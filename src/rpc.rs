//! JSON-RPC helpers for querying Base node info.

use colored::Colorize;
use serde_json::json;

use crate::types::{JsonRpcRequest, JsonRpcResponse};

/// Make a JSON-RPC call to the Base node.
pub async fn call<T: serde::de::DeserializeOwned>(
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> eyre::Result<T> {
    let client = reqwest::Client::new();
    let req = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method,
        params,
    };
    let resp: JsonRpcResponse<T> = client.post(rpc_url).json(&req).send().await?.json().await?;

    if let Some(err) = resp.error {
        eyre::bail!("RPC error {}: {}", err.code, err.message);
    }
    resp.result.ok_or_else(|| eyre::eyre!("Empty RPC response"))
}

/// Display chain info.
pub async fn info(rpc_url: &str) -> eyre::Result<()> {
    println!("{}", "Base Chain Info".bold().cyan());
    println!("{}", "─".repeat(50));

    // Chain ID
    let chain_id: String = call(rpc_url, "eth_chainId", json!([])).await?;
    let chain_id_num = u64::from_str_radix(chain_id.trim_start_matches("0x"), 16).unwrap_or(0);
    let chain_name = match chain_id_num {
        8453 => "Base Mainnet",
        84532 => "Base Sepolia",
        _ => "Unknown",
    };
    println!(
        "  {} {} ({})",
        "Chain:".bold(),
        chain_name.green(),
        chain_id
    );

    // Latest block
    let block: serde_json::Value =
        call(rpc_url, "eth_getBlockByNumber", json!(["latest", false])).await?;

    if let Some(number) = block.get("number").and_then(|n| n.as_str()) {
        let num =
            u64::from_str_radix(number.trim_start_matches("0x"), 16).unwrap_or(0);
        println!("  {} {}", "Block:".bold(), num.to_string().yellow());
    }

    if let Some(timestamp) = block.get("timestamp").and_then(|t| t.as_str()) {
        let ts = u64::from_str_radix(timestamp.trim_start_matches("0x"), 16).unwrap_or(0);
        let dt = chrono::DateTime::from_timestamp(ts as i64, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_default();
        println!("  {} {}", "Time:".bold(), dt);
    }

    if let Some(gas_used) = block.get("gasUsed").and_then(|g| g.as_str()) {
        let gas = u64::from_str_radix(gas_used.trim_start_matches("0x"), 16).unwrap_or(0);
        println!("  {} {}", "Gas Used:".bold(), format_gas(gas));
    }

    if let Some(base_fee) = block.get("baseFeePerGas").and_then(|b| b.as_str()) {
        let fee = u64::from_str_radix(base_fee.trim_start_matches("0x"), 16).unwrap_or(0);
        println!(
            "  {} {:.4} Gwei",
            "Base Fee:".bold(),
            fee as f64 / 1e9
        );
    }

    if let Some(tx_count) = block.get("transactions").and_then(|t| t.as_array()) {
        println!("  {} {}", "Txns:".bold(), tx_count.len());
    }

    println!("  {} {}", "RPC:".bold(), rpc_url.dimmed());

    // Try to detect flashblocks support
    println!();
    println!("{}", "Flashblocks Status".bold().cyan());
    println!("{}", "─".repeat(50));

    // Check pending block (flashblocks show up here)
    match call::<serde_json::Value>(rpc_url, "eth_getBlockByNumber", json!(["pending", false]))
        .await
    {
        Ok(pending) => {
            if let Some(number) = pending.get("number").and_then(|n| n.as_str()) {
                let num =
                    u64::from_str_radix(number.trim_start_matches("0x"), 16).unwrap_or(0);
                println!(
                    "  {} Block {} in progress",
                    "Pending:".bold(),
                    num.to_string().yellow()
                );
            }
            if let Some(txs) = pending.get("transactions").and_then(|t| t.as_array()) {
                println!(
                    "  {} {} transactions pre-confirmed",
                    "Txns:".bold(),
                    txs.len().to_string().green()
                );
            }
        }
        Err(_) => {
            println!(
                "  {} Pending block not available (flashblocks may not be enabled on this RPC)",
                "⚠".yellow()
            );
        }
    }

    Ok(())
}

fn format_gas(gas: u64) -> String {
    if gas >= 1_000_000 {
        format!("{:.2}M", gas as f64 / 1_000_000.0)
    } else if gas >= 1_000 {
        format!("{:.1}K", gas as f64 / 1_000.0)
    } else {
        gas.to_string()
    }
}
