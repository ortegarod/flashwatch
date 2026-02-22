//! Flashblock streaming — subscribe to newFlashblocks and pendingLogs.

use chrono::Utc;
use colored::Colorize;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info};

use crate::format::OutputFormat;
use crate::types::{Flashblock, JsonRpcNotification};

/// Subscribe to `newFlashblocks` and stream them.
pub async fn run(
    ws_url: &str,
    full_txs: bool,
    limit: u64,
    format: &OutputFormat,
) -> eyre::Result<()> {
    info!("Connecting to {}", ws_url);
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected. Subscribing to newFlashblocks...");

    // Send eth_subscribe for newFlashblocks
    let subscribe = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["newFlashblocks"]
    });
    ws.send(Message::Text(subscribe.to_string().into())).await?;

    // Read subscription confirmation
    if let Some(Ok(msg)) = ws.next().await {
        debug!("Subscription response: {}", msg);
    }

    let mut count = 0u64;

    while let Some(Ok(msg)) = ws.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            Message::Ping(_) => continue,
            Message::Pong(_) => continue,
            Message::Close(_) => {
                info!("WebSocket closed by server");
                break;
            }
            _ => continue,
        };

        // Parse as subscription notification
        let notification: JsonRpcNotification = match serde_json::from_str(&text) {
            Ok(n) => n,
            Err(e) => {
                debug!("Non-notification message: {} ({})", text, e);
                continue;
            }
        };

        if let Some(params) = notification.params {
            let flashblock: Flashblock = match serde_json::from_value(params.result) {
                Ok(fb) => fb,
                Err(e) => {
                    error!("Failed to parse flashblock: {}", e);
                    continue;
                }
            };

            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string(&flashblock)?);
                }
                OutputFormat::Pretty => {
                    print_flashblock(&flashblock, full_txs);
                }
            }

            count += 1;
            if limit > 0 && count >= limit {
                info!("Reached limit of {} flashblocks", limit);
                break;
            }
        }
    }

    Ok(())
}

/// Subscribe to `pendingLogs` with optional filters.
pub async fn logs(
    ws_url: &str,
    address: Option<String>,
    topic: Option<String>,
) -> eyre::Result<()> {
    info!("Connecting to {}", ws_url);
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected. Subscribing to pendingLogs...");

    // Build filter params
    let mut filter = json!({});
    if let Some(addr) = &address {
        filter["address"] = json!(addr);
    }
    if let Some(t) = &topic {
        filter["topics"] = json!([[t]]);
    }

    let subscribe = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["pendingLogs", filter]
    });
    ws.send(Message::Text(subscribe.to_string().into())).await?;

    // Read subscription confirmation
    if let Some(Ok(msg)) = ws.next().await {
        debug!("Subscription response: {}", msg);
    }

    println!(
        "{} Streaming pending logs{}{}",
        "◉".green(),
        address
            .as_ref()
            .map(|a| format!(" for {}", a.dimmed()))
            .unwrap_or_default(),
        topic
            .as_ref()
            .map(|t| format!(" topic {}", t.dimmed()))
            .unwrap_or_default(),
    );

    while let Some(Ok(msg)) = ws.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => break,
            _ => continue,
        };

        let notification: JsonRpcNotification = match serde_json::from_str(&text) {
            Ok(n) => n,
            Err(_) => continue,
        };

        if let Some(params) = notification.params {
            let log = params.result;

            let addr = log.get("address").and_then(|a| a.as_str()).unwrap_or("?");
            let topics: Vec<&str> = log
                .get("topics")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let block = log
                .get("blockNumber")
                .and_then(|b| b.as_str())
                .unwrap_or("pending");
            let tx = log
                .get("transactionHash")
                .and_then(|t| t.as_str())
                .unwrap_or("?");

            println!(
                "{} {} {} {} topic0={}",
                Utc::now().format("%H:%M:%S%.3f").to_string().dimmed(),
                block.yellow(),
                &addr[..10].cyan(),
                &tx[..10].dimmed(),
                topics.first().map(|t| &t[..10]).unwrap_or("none").magenta(),
            );
        }
    }

    Ok(())
}

fn print_flashblock(fb: &Flashblock, full_txs: bool) {
    let now = Utc::now().format("%H:%M:%S%.3f");
    let block_num = fb.block_number().map(|n| n.to_string()).unwrap_or("?".into());
    let tx_count = fb.tx_count();
    let gas = fb
        .gas_used_val()
        .map(|g| {
            if g >= 1_000_000 {
                format!("{:.2}M", g as f64 / 1_000_000.0)
            } else {
                format!("{}K", g / 1000)
            }
        })
        .unwrap_or("?".into());
    let base_fee = fb
        .base_fee_gwei()
        .map(|f| format!("{:.4}", f))
        .unwrap_or("?".into());

    println!(
        "{} {} block={} txs={} gas={} base_fee={}gwei",
        now.to_string().dimmed(),
        "⚡".yellow(),
        block_num.cyan(),
        tx_count.to_string().green(),
        gas.yellow(),
        base_fee.magenta(),
    );

    if full_txs {
        if let Some(serde_json::Value::Array(txs)) = &fb.transactions {
            for (i, tx) in txs.iter().enumerate() {
                if let Some(hash) = tx.as_str() {
                    println!("    {} {}", format!("[{}]", i).dimmed(), hash.dimmed());
                } else if let Some(hash) = tx.get("hash").and_then(|h| h.as_str()) {
                    let from = tx.get("from").and_then(|f| f.as_str()).unwrap_or("?");
                    let to = tx
                        .get("to")
                        .and_then(|t| t.as_str())
                        .unwrap_or("(create)");
                    println!(
                        "    {} {} {} → {}",
                        format!("[{}]", i).dimmed(),
                        &hash[..10].dimmed(),
                        &from[..10].cyan(),
                        &to[..10].green(),
                    );
                }
            }
        }
    }
}
