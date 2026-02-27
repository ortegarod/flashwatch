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

    let mut req = client.post(url).json(alert);

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
