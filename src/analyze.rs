//! Transaction lifecycle tracking — from submission to flashblock to canonical block.

use std::time::{Duration, Instant};

use colored::Colorize;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

use crate::rpc;
use crate::types::{Flashblock, JsonRpcNotification};

/// Track a transaction through its lifecycle.
pub async fn track(ws_url: &str, rpc_url: &str, tx_hash: &str) -> eyre::Result<()> {
    println!(
        "{} Tracking transaction {}",
        "🔍".to_string(),
        tx_hash.cyan()
    );
    println!("{}", "─".repeat(60));

    // First check if it's already confirmed
    let receipt: Option<serde_json::Value> =
        rpc::call(rpc_url, "eth_getTransactionReceipt", json!([tx_hash]))
            .await
            .ok();

    if let Some(receipt) = receipt {
        let block = receipt
            .get("blockNumber")
            .and_then(|b| b.as_str())
            .unwrap_or("?");
        let status = receipt
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("?");
        let gas = receipt
            .get("gasUsed")
            .and_then(|g| g.as_str())
            .unwrap_or("?");

        let status_display = if status == "0x1" {
            "✅ Success".green()
        } else {
            "❌ Failed".red()
        };

        println!("  {} Already confirmed in block {}", status_display, block);
        println!("  {} {}", "Gas used:".bold(), gas);
        return Ok(());
    }

    // Not confirmed yet — watch for it in flashblocks
    println!("  {} Not yet confirmed. Watching flashblocks...", "⏳".yellow());

    let (mut ws, _) = connect_async(ws_url).await?;

    let subscribe = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_subscribe",
        "params": ["newFlashblocks"]
    });
    ws.send(Message::Text(subscribe.to_string().into())).await?;

    if let Some(Ok(msg)) = ws.next().await {
        debug!("Subscription response: {}", msg);
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(120);
    let tx_hash_lower = tx_hash.to_lowercase();

    while let Some(Ok(msg)) = ws.next().await {
        if start.elapsed() > timeout {
            println!("  {} Timeout after 120s", "⏰".red());
            break;
        }

        let text = match msg {
            Message::Text(t) => t.to_string(),
            _ => continue,
        };

        let notification: JsonRpcNotification = match serde_json::from_str(&text) {
            Ok(n) => n,
            Err(_) => continue,
        };

        if let Some(params) = notification.params {
            let flashblock: Flashblock = match serde_json::from_value(params.result) {
                Ok(fb) => fb,
                Err(_) => continue,
            };

            // Check if our tx is in this flashblock
            if let Some(serde_json::Value::Array(txs)) = &flashblock.transactions {
                let found = txs.iter().any(|tx| {
                    let hash = tx.as_str().unwrap_or(
                        tx.get("hash").and_then(|h| h.as_str()).unwrap_or(""),
                    );
                    hash.to_lowercase() == tx_hash_lower
                });

                if found {
                    let elapsed = start.elapsed();
                    let block_num = flashblock
                        .block_number()
                        .map(|n| n.to_string())
                        .unwrap_or("?".into());

                    println!(
                        "  {} Found in flashblock! block={} after {:.0}ms",
                        "⚡ Pre-confirmed".green().bold(),
                        block_num.cyan(),
                        elapsed.as_millis(),
                    );
                    println!(
                        "    {} {} txs in this flashblock",
                        "Context:".dimmed(),
                        txs.len(),
                    );

                    // Now wait for canonical confirmation
                    info!("Waiting for canonical block confirmation...");
                    println!("  {} Waiting for canonical block...", "⏳".yellow());

                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if let Ok(receipt) = rpc::call::<serde_json::Value>(
                            rpc_url,
                            "eth_getTransactionReceipt",
                            json!([tx_hash]),
                        )
                        .await
                        {
                            let total = start.elapsed();
                            let block = receipt
                                .get("blockNumber")
                                .and_then(|b| b.as_str())
                                .unwrap_or("?");
                            println!(
                                "  {} Canonical in block {} after {:.1}s total",
                                "✅ Confirmed".green().bold(),
                                block.cyan(),
                                total.as_secs_f64(),
                            );
                            break;
                        }

                        if start.elapsed() > timeout {
                            println!("  {} Timeout waiting for canonical confirmation", "⏰".red());
                            break;
                        }
                    }

                    break;
                }
            }
        }
    }

    Ok(())
}
