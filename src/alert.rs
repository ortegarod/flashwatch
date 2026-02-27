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
    let mut retry_delay = 2u64;

    loop {
        match connect_and_stream(ws_url, &mut engine, json_output, &http_client).await {
            Ok(()) => {
                info!("Stream ended cleanly");
                break;
            }
            Err(e) => {
                warn!("Connection lost: {}. Reconnecting in {}s...", e, retry_delay);
                tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;
                retry_delay = (retry_delay * 2).min(30); // exponential backoff, max 30s
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
    info!("Connected — watching for alerts...");

    let mut current_block: Option<u64> = None;
    let mut alert_count = 0u64;

    while let Some(Ok(msg)) = ws.next().await {
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
                            fire_webhook(client, &engine.config.rules, &alert).await;
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

pub async fn fire_webhook_pub(client: &reqwest::Client, rules: &[crate::rules::Rule], alert: &Alert) {
    fire_webhook(client, rules, alert).await;
}

async fn fire_webhook(client: &reqwest::Client, rules: &[crate::rules::Rule], alert: &Alert) {
    let webhook_url = rules.iter()
        .find(|r| r.name == alert.rule_name)
        .and_then(|r| r.webhook.as_ref());

    let url = match webhook_url {
        Some(u) => u,
        None => return,
    };

    // Build the OpenClaw /hooks/agent payload.
    // The message field is the full prompt the isolated agent session receives.
    let message = build_agent_message(alert);
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
fn build_agent_message(alert: &Alert) -> String {
    // Well-known Base/Ethereum addresses. Add your own as you discover them.
    let known: &[(&str, &str)] = &[
        ("0x71660c4005ba85c37ccec55d0c4493e66fe775d3", "Coinbase Hot Wallet"),
        ("0xa9d1e08c7793af67e9d92fe308d5697fb81d3e43", "Coinbase Cold Storage"),
        ("0x503828976d22510aad0201ac7ec88293211d23da", "Coinbase 2"),
        ("0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740", "Coinbase 3"),
        ("0x28c6c06298d514db089934071355e5743bf21d60", "Binance Hot Wallet"),
        ("0x21a31ee1afc51d94c2efccaa2092ad1028285549", "Binance Cold Wallet"),
        ("0x3154cf16ccdb4c6d922629664174b904d80f2c35", "Base Bridge (L1)"),
        ("0x4200000000000000000000000000000000000010", "Base L2 Bridge"),
        ("0x2626664c2603336e57b271c5c0b26f421741e481", "Uniswap V3 Router (Base)"),
        ("0x198ef1ec325a96cc354c7266a038be8b5c558f67", "Uniswap Universal Router (Base)"),
        ("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", "USDC (Base)"),
    ];

    let label = |addr: &str| -> Option<&str> {
        let lower = addr.to_lowercase();
        known.iter().find(|(k, _)| *k == lower).map(|(_, v)| *v)
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

    let submolt = std::env::var("FLASHWATCH_MOLTBOOK_SUBMOLT")
        .unwrap_or_else(|_| "basewhales".to_string());

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
    lines.push("== YOUR JOB ==".to_string());
    lines.push(String::new());
    lines.push("1. RESEARCH the wallets if they're unknown.".to_string());
    lines.push("   - Fetch the Basescan address pages above using web_fetch".to_string());
    lines.push("   - Look for tags, contract names, ENS names, transaction patterns".to_string());
    lines.push("   - Is this a known exchange, protocol, whale, or DAO?".to_string());
    lines.push("   - Is it a contract or an EOA? What has this address done before?".to_string());
    lines.push(String::new());
    lines.push("2. INTERPRET the movement. Ask yourself:".to_string());
    lines.push("   - Exchange deposit/withdrawal? → signals buy/sell intent".to_string());
    lines.push("   - Cold storage rotation? → routine treasury management, low signal".to_string());
    lines.push("   - Bridge deposit? → capital moving cross-chain, where and why?".to_string());
    lines.push("   - Unknown wallet moving big size? → who is this and why now?".to_string());
    lines.push("   - Anything unusual about the timing, size, or counterparty?".to_string());
    lines.push(String::new());
    lines.push("3. WRITE a Moltbook post. Keep it under 280 characters + link. Format:".to_string());
    lines.push("   [emoji] [value] ETH [what happened] on Base".to_string());
    lines.push("   [one-line interpretation — confident, specific, have a take]".to_string());
    lines.push("   🔗 [tx link]".to_string());
    lines.push(String::new());
    lines.push("   Emoji guide: 🐋 huge (500+ ETH) | 🦈 large (200-500) | 🔥 interesting (100-200)".to_string());
    lines.push("   Be direct. If you don't know who the wallet is, say so honestly.".to_string());
    lines.push("   Don't make up labels. Don't pad with generic phrases.".to_string());
    lines.push(String::new());
    lines.push(format!("4. POST to Moltbook using the API (posting to /m/{}):", submolt));
    lines.push("   Key at: ~/.config/moltbook/credentials.json (use exec, do NOT read the file directly)".to_string());
    lines.push("   MOLTKEY=$(python3 -c \"import json,os; d=json.load(open(os.path.expanduser('~/.config/moltbook/credentials.json'))); print(d['api_key'])\")".to_string());
    lines.push(format!("   RESPONSE=$(curl -s -X POST \"https://www.moltbook.com/api/v1/posts\" \\"));
    lines.push("     -H \"Authorization: Bearer $MOLTKEY\" \\".to_string());
    lines.push("     -H \"Content-Type: application/json\" \\".to_string());
    lines.push(format!("     -d '{{\"submolt\":\"{}\",\"title\":\"[your title]\",\"content\":\"[your post]\"}}\')", submolt));
    lines.push("   echo \"$RESPONSE\"".to_string());
    lines.push(String::new());
    lines.push("   Rate limit: 1 post per 30 min. If you get a 429, log it and skip — don't retry.".to_string());
    lines.push(String::new());
    lines.push("5. CHECK FOR VERIFICATION CHALLENGE:".to_string());
    lines.push("   Some communities require a math verification to prevent spam. Check the response:".to_string());
    lines.push("   CODE=$(echo \"$RESPONSE\" | python3 -c \"import json,sys; d=json.load(sys.stdin); print(d.get('post',{}).get('verification',{}).get('verification_code',''))\")".to_string());
    lines.push("   CHALLENGE=$(echo \"$RESPONSE\" | python3 -c \"import json,sys; d=json.load(sys.stdin); print(d.get('post',{}).get('verification',{}).get('challenge_text',''))\")".to_string());
    lines.push(String::new());
    lines.push("   If CODE is non-empty, you must verify:".to_string());
    lines.push("   - Read the CHALLENGE text carefully — it's an obfuscated math word problem".to_string());
    lines.push("   - Decode the words and solve the arithmetic, format to 2 decimal places (e.g. \"25.00\")".to_string());
    lines.push("   - Submit: curl -s -X POST \"https://www.moltbook.com/api/v1/verify\" \\".to_string());
    lines.push("       -H \"Authorization: Bearer $MOLTKEY\" \\".to_string());
    lines.push("       -H \"Content-Type: application/json\" \\".to_string());
    lines.push("       -d '{\"verification_code\":\"'$CODE'\",\"answer\":\"[your answer]\"}'".to_string());

    lines.join("\n")
}
