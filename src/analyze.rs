//! Transaction lifecycle tracking.

use std::io::Read;
use std::time::{Duration, Instant};

use colored::Colorize;
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

use crate::rpc;
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

/// Track a transaction through its lifecycle.
pub async fn track(ws_url: &str, rpc_url: &str, tx_hash: &str) -> eyre::Result<()> {
    println!(
        "{} Tracking transaction {}",
        "🔍".to_string(),
        tx_hash.cyan()
    );
    println!("{}", "─".repeat(60));

    // First check if already confirmed
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

        println!("  {} Confirmed in block {}", status_display, block);
        println!("  {} {}", "Gas used:".bold(), gas);
        return Ok(());
    }

    // Watch flashblocks feed for it
    println!("  {} Not yet confirmed. Watching flashblocks...", "⏳".yellow());

    let (mut ws, _) = connect_async(ws_url).await?;
    let start = Instant::now();
    let timeout = Duration::from_secs(120);
    let tx_hash_lower = tx_hash.to_lowercase();

    while let Some(Ok(msg)) = ws.next().await {
        if start.elapsed() > timeout {
            println!("  {} Timeout after 120s", "⏰".red());
            break;
        }

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

        // Check transactions in this diff — they may be raw RLP bytes
        for tx in &fb.diff.transactions {
            let tx_str = tx.as_str().unwrap_or("");
            // Raw transactions won't have a hash directly — we'd need to decode RLP
            // For now check if the hash appears in any string representation
            if tx_str.to_lowercase().contains(&tx_hash_lower[2..]) {
                let elapsed = start.elapsed();
                println!(
                    "  {} Found in flashblock #{} after {:.0}ms",
                    "⚡ Pre-confirmed".green().bold(),
                    fb.index,
                    elapsed.as_millis(),
                );

                // Wait for canonical confirmation
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
                            "  {} Block {} — {:.1}s total",
                            "✅ Canonical".green().bold(),
                            block.cyan(),
                            total.as_secs_f64(),
                        );
                        break;
                    }
                    if start.elapsed() > timeout {
                        println!("  {} Timeout waiting for canonical", "⏰".red());
                        break;
                    }
                }
                return Ok(());
            }
        }
    }

    Ok(())
}
