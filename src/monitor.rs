//! Real-time flashblock metrics monitor.

use std::io::Read;
use std::time::Instant;

use colored::Colorize;
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

use crate::types::{FlashblockMessage, FlashblockMetrics};

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

/// Run the live monitor display.
pub async fn run(ws_url: &str, refresh_ms: u64) -> eyre::Result<()> {
    info!("Connecting to {}", ws_url);
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected — monitoring flashblocks...");

    let mut metrics = FlashblockMetrics::default();
    let start = Instant::now();
    let mut last_print = Instant::now();
    let mut first_print = true;

    println!("{}", "flashwatch monitor — Ctrl+C to exit".bold().cyan());
    println!();
    // Reserve lines for the display
    for _ in 0..8 {
        println!();
    }

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
            Err(e) => {
                debug!("Failed to parse: {}", e);
                continue;
            }
        };

        metrics.update(&fb);

        // Calculate rate
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            metrics.flashblocks_per_second = metrics.total_flashblocks as f64 / elapsed;
        }

        // Refresh display at interval
        if first_print || last_print.elapsed().as_millis() >= refresh_ms as u128 {
            print_metrics(&metrics);
            last_print = Instant::now();
            first_print = false;
        }
    }

    Ok(())
}

fn print_metrics(m: &FlashblockMetrics) {
    // Move cursor up and clear
    print!("\x1B[8A\x1B[J");

    let block_num = m
        .current_block
        .block_number
        .map(|n| n.to_string())
        .unwrap_or("—".into());
    let base_fee = m
        .current_block
        .base_fee_gwei
        .map(|f| format!("{:.4}", f))
        .unwrap_or("—".into());
    let avg_tx = if m.total_flashblocks > 0 {
        m.total_transactions as f64 / m.total_flashblocks as f64
    } else {
        0.0
    };
    let avg_gas = if m.total_flashblocks > 0 {
        m.total_gas_used as f64 / m.total_flashblocks as f64
    } else {
        0.0
    };

    println!(
        "  {} {}  {} {}",
        "Block:".bold(),
        block_num.cyan(),
        "Base Fee:".bold(),
        format!("{} gwei", base_fee).magenta(),
    );
    println!(
        "  {} {} total  {} in current block",
        "Flashblocks:".bold(),
        m.total_flashblocks.to_string().yellow(),
        m.current_block.flashblock_count.to_string().green(),
    );
    println!(
        "  {} {:.1}/s",
        "Rate:".bold(),
        m.flashblocks_per_second,
    );
    println!(
        "  {} {} total  avg {:.1}/fb",
        "Transactions:".bold(),
        m.total_transactions.to_string().green(),
        avg_tx,
    );
    println!(
        "  {} {} in current block",
        "Block Txns:".bold(),
        m.current_block.total_tx_count.to_string().green(),
    );
    println!(
        "  {} avg {:.0} per flashblock",
        "Gas:".bold(),
        avg_gas,
    );
    println!(
        "  {} {} blocks seen",
        "Blocks:".bold(),
        m.blocks_seen.to_string().cyan(),
    );
    println!(
        "  {} {}ms ago",
        "Last:".bold(),
        m.last_received
            .map(|t| t.elapsed().as_millis().to_string())
            .unwrap_or("—".into())
            .dimmed(),
    );
}
