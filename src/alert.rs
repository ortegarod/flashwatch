//! Alert subcommand — stream flashblocks, match rules, log/webhook on hits.

use std::io::Read;

use chrono::Utc;
use colored::Colorize;
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

use crate::decode;
use crate::rules::{Alert, RuleEngine};
use crate::types::FlashblockMessage;

fn decode_message(data: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(data) {
        if text.trim_start().starts_with('{') {
            return Some(text.to_owned());
        }
    }
    let mut decompressor = brotli::Decompressor::new(data, 4096);
    let mut decompressed = Vec::new();
    if decompressor.read_to_end(&mut decompressed).is_ok() {
        return String::from_utf8(decompressed).ok();
    }
    None
}

pub async fn run(ws_url: &str, rules_path: &str, json_output: bool) -> eyre::Result<()> {
    let rules_str = std::fs::read_to_string(rules_path)?;
    let mut engine = RuleEngine::from_toml(&rules_str)?;

    let rule_count = engine.config.rules.iter().filter(|r| r.enabled).count();
    info!("Loaded {} active rules from {}", rule_count, rules_path);

    for rule in &engine.config.rules {
        if rule.enabled {
            info!(
                "  ✓ {} → {}",
                rule.name,
                rule.webhook.as_deref().unwrap_or("(log only)")
            );
        }
    }

    // Collect webhook URLs for the HTTP client
    let has_webhooks = engine.config.rules.iter().any(|r| r.webhook.is_some());
    let http_client = if has_webhooks {
        Some(reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?)
    } else {
        None
    };

    info!("Connecting to {}", ws_url);
    const RECONNECT_PAUSE: std::time::Duration = std::time::Duration::from_secs(2);

    loop {
        match connect_and_stream(ws_url, &mut engine, json_output, &http_client).await {
            Ok(()) => {
                info!("Stream ended cleanly");
                break;
            }
            Err(e) => {
                warn!("Connection lost: {}. Reconnecting in {}s...", e, RECONNECT_PAUSE.as_secs());
                tokio::time::sleep(RECONNECT_PAUSE).await;
            }
        }
    }

    Ok(())
}

async fn connect_and_stream(
    ws_url: &str,
    engine: &mut RuleEngine,
    json_output: bool,
    http_client: &Option<reqwest::Client>,
) -> eyre::Result<()> {
    let (mut ws, _) = connect_async(ws_url).await?;

    // TCP keepalive — OS-level dead connection detection
    if let tokio_tungstenite::MaybeTlsStream::Rustls(tls) = ws.get_ref() {
        let tcp: &tokio::net::TcpStream = tls.get_ref().0;
        let sock = socket2::SockRef::from(tcp);
        let keepalive = socket2::TcpKeepalive::new()
            .with_time(std::time::Duration::from_secs(10))
            .with_interval(std::time::Duration::from_secs(5))
            .with_retries(3);
        sock.set_tcp_keepalive(&keepalive)?;
    }

    info!("Connected — watching for alerts...");

    let mut current_block: Option<u64> = None;
    let mut alert_count = 0u64;

    const STALE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    loop {
        let msg = tokio::select! {
            msg = ws.next() => {
                match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                }
            }
            _ = tokio::time::sleep(STALE_TIMEOUT) => {
                return Err(eyre::eyre!(
                    "No data received from upstream in {}s — connection stale",
                    STALE_TIMEOUT.as_secs()
                ));
            }
        };

        let data = match msg {
            Message::Text(t) => t.as_bytes().to_vec(),
            Message::Binary(b) => b.to_vec(),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => break,
            _ => continue,
        };

        let text = match decode_message(&data) {
            Some(t) => t,
            None => continue,
        };

        let fb: FlashblockMessage = match serde_json::from_str(&text) {
            Ok(fb) => fb,
            Err(_) => continue,
        };

        let block_number = fb.block_number().or(current_block);
        if fb.block_number().is_some() {
            current_block = fb.block_number();
        }

        // Decode each transaction and check rules
        for tx_val in &fb.diff.transactions {
            if let Some(tx_hex) = tx_val.as_str() {
                if let Some(decoded) = decode::decode_raw_tx(tx_hex) {
                    let alerts = engine.check(&decoded, block_number, fb.index);
                    for alert in alerts {
                        alert_count += 1;

                        if json_output {
                            if let Ok(json) = serde_json::to_string(&alert) {
                                println!("{}", json);
                            }
                        } else {
                            print_alert(&alert, alert_count);
                        }

                        // Fire webhook if configured
                        if let Some(client) = http_client {
                            fire_webhook(client, &engine.config, &alert).await;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_alert(alert: &Alert, count: u64) {
    let now = Utc::now().format("%H:%M:%S%.3f");
    let block = alert.block_number
        .map(|n| n.to_string())
        .unwrap_or("?".into());

    let value = if alert.tx.value_eth > 0.001 {
        format!("{:.4} ETH", alert.tx.value_eth).green().to_string()
    } else {
        String::new()
    };

    let target = alert.tx.to_label.as_deref()
        .unwrap_or(alert.tx.to.as_deref().unwrap_or("?"));

    let action = alert.tx.action.as_deref().unwrap_or("");

    println!(
        "{} {} #{} [{}] block {} fb{} {} → {} {} {}",
        now.to_string().dimmed(),
        "🚨".to_string(),
        count.to_string().bold(),
        alert.rule_name.yellow(),
        block.cyan(),
        alert.flashblock_index,
        action.dimmed(),
        target.bold(),
        value,
        alert.tx.category.dimmed(),
    );
}

pub async fn fire_webhook_pub(client: &reqwest::Client, config: &crate::rules::RulesConfig, alert: &Alert) {
    fire_webhook(client, config, alert).await;
}

async fn fire_webhook(client: &reqwest::Client, config: &crate::rules::RulesConfig, alert: &Alert) {
    let webhook_url = config.rules.iter()
        .find(|r| r.name == alert.rule_name)
        .and_then(|r| r.webhook.as_ref());

    let url = match webhook_url {
        Some(u) => u,
        None => return,
    };

    // Build the OpenClaw /hooks/agent payload.
    // The message field is the full prompt the isolated agent session receives.
    let message = build_agent_message(alert, &config.labels);
    let payload = serde_json::json!({
        "message": message,
        "name": "FlashWatch",
        "wakeMode": "now",
        "deliver": false
    });

    let mut req = client.post(url).json(&payload);

    if let Ok(token) = std::env::var("OPENCLAW_HOOKS_TOKEN") {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    match req.send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                debug!("Webhook {} returned {}", url, resp.status());
            }
        }
        Err(e) => {
            debug!("Webhook {} failed: {}", url, e);
        }
    }
}

/// Build the agent message sent to OpenClaw /hooks/agent.
/// This is the full prompt the isolated agent session receives — it tells the
/// agent what happened on-chain and what to do about it.
fn build_agent_message(alert: &Alert, labels: &std::collections::HashMap<String, String>) -> String {
    let label = |addr: &str| -> Option<&str> {
        labels.get(&addr.to_lowercase()).map(|s| s.as_str())
    };

    let fmt_addr = |addr: Option<&str>| -> String {
        match addr {
            None => "unknown".to_string(),
            Some(a) => match label(a) {
                Some(l) => format!("{} ({})", a, l),
                None => a.to_string(),
            }
        }
    };

    let tx = &alert.tx;
    let from_str = fmt_addr(tx.from.as_deref());
    let to_str = match &tx.to_label {
        Some(l) => format!("{} ({})", tx.to.as_deref().unwrap_or("unknown"), l),
        None => fmt_addr(tx.to.as_deref()),
    };
    let value = format!("{:.2} ETH", tx.value_eth);
    let block = match alert.block_number {
        Some(n) => format!("block {} fb{}", n, alert.flashblock_index),
        None => String::new(),
    };
    let tx_link = tx.hash.as_ref()
        .map(|h| format!("https://basescan.org/tx/{}", h));
    let from_basescan = tx.from.as_ref()
        .filter(|_| label(tx.from.as_deref().unwrap_or("")).is_none())
        .map(|a| format!("https://basescan.org/address/{}", a));
    let to_basescan = tx.to.as_ref()
        .filter(|_| tx.to_label.is_none() && label(tx.to.as_deref().unwrap_or("")).is_none())
        .map(|a| format!("https://basescan.org/address/{}", a));

    let mut lines = vec![
        format!("[FlashWatch Alert — Base Mainnet]"),
        format!("{} | Rule: {} | {}", value, alert.rule_name, block),
        format!("From: {}", from_str),
        format!("To:   {}", to_str),
    ];
    if let Some(ref link) = tx_link {
        lines.push(format!("Tx: {}", link));
    }
    if let Some(ref link) = from_basescan {
        lines.push(format!("From profile: {}", link));
    }
    if let Some(ref link) = to_basescan {
        lines.push(format!("To profile:   {}", link));
    }

    lines.push(String::new());
    lines.push("Post this alert to Moltbook /m/basewhales using your Moltbook skill. Research the wallets above, then POST to https://www.moltbook.com/api/v1/posts with submolt=basewhales.\n\nPOST FORMAT — strictly follow this order:\n1. First line: your analysis/hook — what this movement means, who these wallets are, what to watch for. This is the preview people see. Make it worth reading.\n2. Then: the raw data (ETH amount, from/to wallets with labels if known, basescan tx link).\n\nExample:\nUnknown wallet moving serious size to Base. New whale entering the ecosystem — watch for DeFi activity.\n\n1,200 ETH transferred on Base\n0xabc...123 (unknown) → 0xdef...456 (unknown)\n🔗 https://basescan.org/tx/0x...\n\nIf the response includes a verification challenge, solve the math and POST to /api/v1/verify with the verification_code and your answer. CRITICAL: The answer must be ONLY the number with 2 decimal places (e.g., '30.00'), no units like 'Newton' or 'N', no extra text.".to_string());

    lines.join("\n")
}
