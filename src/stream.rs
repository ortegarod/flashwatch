//! Flashblock streaming — connect to Base's raw flashblock WebSocket feed.

use std::io::Read;

use chrono::Utc;
use colored::Colorize;
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

use crate::format::OutputFormat;
use crate::types::FlashblockMessage;

/// Decode a WebSocket message — may be plain JSON text or brotli-compressed binary.
fn decode_message(data: &[u8]) -> Option<String> {
    // Try plain text first
    if let Ok(text) = std::str::from_utf8(data) {
        if text.trim_start().starts_with('{') {
            return Some(text.to_owned());
        }
    }
    // Try brotli decompression
    let mut decompressor = brotli::Decompressor::new(data, 4096);
    let mut decompressed = Vec::new();
    if decompressor.read_to_end(&mut decompressed).is_ok() {
        return String::from_utf8(decompressed).ok();
    }
    None
}

/// Connect to the flashblocks WebSocket and stream messages.
/// The Base feed is a raw push — no subscription needed. Just connect and receive.
pub async fn run(
    ws_url: &str,
    full_txs: bool,
    limit: u64,
    format: &OutputFormat,
) -> eyre::Result<()> {
    info!("Connecting to {}", ws_url);
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected — receiving flashblocks...");

    let mut count = 0u64;
    let mut current_block_num: Option<u64> = None;

    while let Some(Ok(msg)) = ws.next().await {
        let data = match msg {
            Message::Text(t) => t.as_bytes().to_vec(),
            Message::Binary(b) => b.to_vec(),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => {
                info!("WebSocket closed by server");
                break;
            }
            _ => continue,
        };

        let text = match decode_message(&data) {
            Some(t) => t,
            None => {
                debug!("Could not decode message ({} bytes)", data.len());
                continue;
            }
        };

        let fb: FlashblockMessage = match serde_json::from_str(&text) {
            Ok(fb) => fb,
            Err(e) => {
                debug!("Failed to parse JSON: {} — {}", e, &text[..text.len().min(200)]);
                continue;
            }
        };

        match format {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string(&fb)?);
            }
            OutputFormat::Pretty => {
                print_flashblock(&fb, full_txs, &mut current_block_num);
            }
        }

        count += 1;
        if limit > 0 && count >= limit {
            info!("Reached limit of {} flashblocks", limit);
            break;
        }
    }

    Ok(())
}

/// Subscribe to pendingLogs (requires a JSON-RPC WebSocket endpoint, not the raw feed).
pub async fn logs(
    ws_url: &str,
    address: Option<String>,
    topic: Option<String>,
) -> eyre::Result<()> {
    // pendingLogs requires a JSON-RPC WS endpoint (e.g., from a Base node or Alchemy)
    // not the raw flashblocks feed
    info!("Connecting to {}", ws_url);

    // For the raw flashblocks feed, we can filter transactions/receipts ourselves
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected — filtering logs from flashblock diffs...");

    let addr_filter = address.as_deref().map(|a| a.to_lowercase());
    let topic_filter = topic.as_deref().map(|t| t.to_lowercase());

    println!(
        "{} Streaming logs from flashblocks{}{}",
        "◉".green(),
        addr_filter
            .as_ref()
            .map(|a| format!(" address={}", a.dimmed()))
            .unwrap_or_default(),
        topic_filter
            .as_ref()
            .map(|t| format!(" topic0={}", t.dimmed()))
            .unwrap_or_default(),
    );

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

        // Extract logs from receipts if available
        if let Some(receipts) = &fb.diff.receipts {
            let receipt_list = match receipts {
                serde_json::Value::Array(arr) => arr.clone(),
                _ => continue,
            };

            for receipt in &receipt_list {
                let logs = match receipt.get("logs").and_then(|l| l.as_array()) {
                    Some(l) => l,
                    None => continue,
                };

                for log in logs {
                    let log_addr = log
                        .get("address")
                        .and_then(|a| a.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let topics: Vec<&str> = log
                        .get("topics")
                        .and_then(|t| t.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    // Apply filters
                    if let Some(ref af) = addr_filter {
                        if log_addr != *af {
                            continue;
                        }
                    }
                    if let Some(ref tf) = topic_filter {
                        let matches = topics.iter().any(|t| t.to_lowercase() == *tf);
                        if !matches {
                            continue;
                        }
                    }

                    let tx_hash = log
                        .get("transactionHash")
                        .and_then(|t| t.as_str())
                        .unwrap_or("?");

                    println!(
                        "{} {} {} topic0={}",
                        Utc::now().format("%H:%M:%S%.3f").to_string().dimmed(),
                        &log_addr[..log_addr.len().min(12)].cyan(),
                        &tx_hash[..tx_hash.len().min(12)].dimmed(),
                        topics
                            .first()
                            .map(|t| &t[..t.len().min(12)])
                            .unwrap_or("none")
                            .magenta(),
                    );
                }
            }
        }
    }

    Ok(())
}

fn print_flashblock(fb: &FlashblockMessage, full_txs: bool, current_block: &mut Option<u64>) {
    let now = Utc::now().format("%H:%M:%S%.3f");
    let tx_count = fb.tx_count();
    let gas = fb
        .gas_used()
        .map(|g| {
            if g >= 1_000_000 {
                format!("{:.2}M", g as f64 / 1_000_000.0)
            } else if g >= 1_000 {
                format!("{:.1}K", g as f64 / 1_000.0)
            } else {
                format!("{}", g)
            }
        })
        .unwrap_or("—".into());

    // Print block header on new block
    if fb.index == 0 {
        let block_num = fb.block_number().map(|n| n.to_string()).unwrap_or("?".into());
        let base_fee = fb
            .base_fee_gwei()
            .map(|f| format!("{:.4} gwei", f))
            .unwrap_or_default();

        if current_block.is_some() {
            println!(); // separator between blocks
        }
        *current_block = fb.block_number();

        println!(
            "{} {} block {} {}",
            now.to_string().dimmed(),
            "█".cyan().bold(),
            block_num.cyan().bold(),
            base_fee.dimmed(),
        );
    }

    // Print flashblock line
    let idx_display = format!("fb{}", fb.index);
    println!(
        "{} {} {} txs={} gas={}",
        now.to_string().dimmed(),
        "⚡".yellow(),
        idx_display.yellow(),
        tx_count.to_string().green(),
        gas,
    );

    if full_txs && !fb.diff.transactions.is_empty() {
        for (i, tx) in fb.diff.transactions.iter().enumerate() {
            if let Some(tx_str) = tx.as_str() {
                // Raw transaction bytes
                println!(
                    "      {} {}…",
                    format!("[{}]", i).dimmed(),
                    &tx_str[..tx_str.len().min(40)].dimmed(),
                );
            } else if let Some(hash) = tx.get("hash").and_then(|h| h.as_str()) {
                let from = tx.get("from").and_then(|f| f.as_str()).unwrap_or("?");
                let to = tx
                    .get("to")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(create)");
                println!(
                    "      {} {} {} → {}",
                    format!("[{}]", i).dimmed(),
                    &hash[..hash.len().min(12)].dimmed(),
                    &from[..from.len().min(12)].cyan(),
                    &to[..to.len().min(12)].green(),
                );
            }
        }
    }
}
