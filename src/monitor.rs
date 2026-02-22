//! Real-time flashblock metrics monitor.

use std::time::Instant;

use colored::Colorize;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info};

use crate::types::{Flashblock, FlashblockMetrics, JsonRpcNotification};

/// Run the live monitor display.
pub async fn run(ws_url: &str, refresh_ms: u64) -> eyre::Result<()> {
    info!("Connecting to {}", ws_url);
    let (mut ws, _) = connect_async(ws_url).await?;
    info!("Connected. Subscribing to newFlashblocks...");

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

    let mut metrics = FlashblockMetrics::default();
    let start = Instant::now();
    let mut last_print = Instant::now();

    println!("{}", "flashwatch monitor — Ctrl+C to exit".bold().cyan());
    println!();

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
            let flashblock: Flashblock = match serde_json::from_value(params.result) {
                Ok(fb) => fb,
                Err(_) => continue,
            };

            metrics.update(&flashblock);

            // Calculate rate
            let elapsed = start.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                metrics.flashblocks_per_second = metrics.total_flashblocks as f64 / elapsed;
            }

            // Refresh display at interval
            if last_print.elapsed().as_millis() >= refresh_ms as u128 {
                print_metrics(&metrics);
                last_print = Instant::now();
            }
        }
    }

    Ok(())
}

fn print_metrics(m: &FlashblockMetrics) {
    // Move cursor up and clear (simple refresh without full TUI)
    print!("\x1B[8A\x1B[J");

    println!(
        "  {} {}",
        "Block:".bold(),
        m.current_block_number.to_string().cyan()
    );
    println!(
        "  {} {} (in current block: {})",
        "Flashblocks:".bold(),
        m.total_flashblocks.to_string().yellow(),
        m.flashblocks_in_current_block.to_string().green(),
    );
    println!(
        "  {} {:.1}/s",
        "Rate:".bold(),
        m.flashblocks_per_second,
    );
    println!(
        "  {} {} (avg {:.1}/fb)",
        "Transactions:".bold(),
        m.total_transactions.to_string().green(),
        m.avg_tx_per_flashblock,
    );
    println!(
        "  {} avg {:.0}",
        "Gas/flashblock:".bold(),
        m.avg_gas_per_flashblock,
    );
    println!(
        "  {} {:.4} Gwei",
        "Base Fee:".bold(),
        m.last_base_fee_gwei,
    );
    println!(
        "  {} {}ms ago",
        "Last seen:".bold(),
        m.last_received
            .map(|t| t.elapsed().as_millis().to_string())
            .unwrap_or("never".into())
            .dimmed(),
    );
    println!();
}
