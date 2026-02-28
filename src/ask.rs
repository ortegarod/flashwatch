//! x402-gated /api/ask — pay USDC on Base, get AI analysis of Base whale activity.
//! Payment verified via facilitator. Question forwarded to OpenClaw /v1/chat/completions.
//!
//! Config env vars:
//!   X402_FACILITATOR_URL  — default: https://facilitator.x402.rs
//!   X402_NETWORK          — default: base
//!   X402_ASSET            — default: 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913 (USDC mainnet)
//!   X402_PAY_TO           — required: wallet address to receive payments
//!   X402_PRICE            — default: 10000 (0.01 USDC in 6-decimal units)
//!   X402_RESOURCE_URL     — default: https://basewhales.com/api/ask
//!   OPENCLAW_PORT         — default: 18789

use std::sync::Arc;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};

use crate::serve::AppState;
use crate::store::AlertQuery;

/// x402 payment configuration — loaded from env vars at startup.
#[derive(Clone, Debug)]
pub struct X402Config {
    pub facilitator_url: String,
    pub network: String,
    pub asset: String,
    pub pay_to: String,
    pub price: String,
    pub resource_url: String,
    pub openclaw_port: u16,
}

impl X402Config {
    pub fn from_env() -> Self {
        let pay_to = std::env::var("X402_PAY_TO").unwrap_or_else(|_| {
            tracing::warn!("X402_PAY_TO not set — /api/ask payments will go to zero address");
            "0x0000000000000000000000000000000000000000".to_string()
        });
        Self {
            facilitator_url: std::env::var("X402_FACILITATOR_URL")
                .unwrap_or_else(|_| "https://facilitator.x402.rs".to_string()),
            network: std::env::var("X402_NETWORK")
                .unwrap_or_else(|_| "base".to_string()),
            asset: std::env::var("X402_ASSET")
                .unwrap_or_else(|_| "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string()),
            pay_to,
            price: std::env::var("X402_PRICE")
                .unwrap_or_else(|_| "10000".to_string()),
            resource_url: std::env::var("X402_RESOURCE_URL")
                .unwrap_or_else(|_| "https://basewhales.com/api/ask".to_string()),
            openclaw_port: std::env::var("OPENCLAW_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(18789),
        }
    }
}

#[derive(Deserialize)]
pub struct AskRequest {
    pub question: String,
}

#[derive(Serialize)]
pub struct AskResponse {
    pub answer: String,
}

/// Main handler — checks x402 payment, then proxies to OpenClaw.
pub async fn ask_handler(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<AskRequest>,
) -> impl IntoResponse {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    // 1. Check for X-Payment header
    let payment_header = headers
        .get("X-Payment")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    match payment_header {
        None => {
            // Return 402 with payment requirements
            payment_required_response(&state.x402)
        }
        Some(payment) => {
            // 2. Verify payment with facilitator
            match verify_payment(&client, &state.x402.facilitator_url, &payment).await {
                Ok(true) => {
                    // 3. Forward to OpenClaw
                    match query_openclaw(&client, &state, &req.question).await {
                        Ok(answer) => (
                            StatusCode::OK,
                            Json(serde_json::json!({ "answer": answer })),
                        ).into_response(),
                        Err(e) => (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({ "error": format!("Agent error: {e}") })),
                        ).into_response(),
                    }
                }
                Ok(false) => payment_required_response(&state.x402),
                Err(e) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({ "error": format!("Facilitator error: {e}") })),
                ).into_response(),
            }
        }
    }
}

/// Returns a 402 Payment Required response with the x402 payment spec.
fn payment_required_response(x402: &X402Config) -> axum::response::Response {
    let description = format!(
        "BaseWhales AI query — {} USDC on {}",
        format_price(&x402.price), x402.network
    );
    let body = serde_json::json!({
        "x402Version": 1,
        "accepts": [{
            "scheme": "exact",
            "network": x402.network,
            "maxAmountRequired": x402.price,
            "resource": x402.resource_url,
            "description": description,
            "mimeType": "application/json",
            "payTo": x402.pay_to,
            "maxTimeoutSeconds": 300,
            "asset": x402.asset,
            "extra": { "name": "USD Coin", "version": "2" }
        }],
        "error": "Payment required"
    });
    (StatusCode::PAYMENT_REQUIRED, Json(body)).into_response()
}

/// Format raw USDC units (6 decimals) as human-readable amount.
fn format_price(raw: &str) -> String {
    raw.parse::<f64>()
        .map(|n| format!("{:.2}", n / 1_000_000.0))
        .unwrap_or_else(|_| raw.to_string())
}

/// Verify payment with the x402 facilitator. Returns true if valid.
async fn verify_payment(client: &reqwest::Client, facilitator_url: &str, payment: &str) -> eyre::Result<bool> {
    let resp = client
        .post(format!("{facilitator_url}/verify"))
        .header("content-type", "application/json")
        .body(payment.to_string())
        .send()
        .await?;

    Ok(resp.status().is_success())
}

/// Build a rich context message for the agent.
fn build_context(state: &AppState) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut ctx = format!(
        "You are BaseWhales — an AI agent that monitors Base L2 flashblocks 24/7 and \
        analyzes whale movements in real time. You are the intelligence behind https://basewhales.com.\n\n\
        Current time (UTC): {}\n\n",
        chrono::DateTime::from_timestamp(now as i64, 0)
            .map(|dt: chrono::DateTime<chrono::Utc>| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );

    // Known wallet labels
    if let Some(ref rc) = state.rules_config {
        if !rc.labels.is_empty() {
            ctx.push_str("Known wallet labels:\n");
            for (addr, label) in &rc.labels {
                ctx.push_str(&format!("  {addr} = {label}\n"));
            }
            ctx.push('\n');
        }
    }

    // Recent alerts from SQLite
    if let Some(ref store) = state.store {
        let query = AlertQuery {
            since_ts: Some(now - 86_400),
            limit: Some(50),
            ..Default::default()
        };
        if let Ok(alerts) = store.query(&query) {
            let total_eth: f64 = alerts.iter()
                .filter_map(|a| a.get("tx").and_then(|t| t.get("value_eth")).and_then(|v| v.as_f64()))
                .sum();
            let biggest = alerts.iter()
                .filter_map(|a| a.get("tx").and_then(|t| t.get("value_eth")).and_then(|v| v.as_f64()))
                .fold(0f64, f64::max);

            ctx.push_str(&format!(
                "Last 24h activity: {} whale alerts detected, {:.1} ETH total moved, \
                largest single move: {:.1} ETH\n\nRecent alerts (newest first):\n",
                alerts.len(), total_eth, biggest
            ));

            for alert in alerts.iter().take(20) {
                let value = alert.get("tx").and_then(|t| t.get("value_eth")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let to_addr = alert.get("tx").and_then(|t| t.get("to")).and_then(|v| v.as_str()).unwrap_or("unknown");
                let to_label = alert.get("tx").and_then(|t| t.get("to_label")).and_then(|v| v.as_str());
                let ts = alert.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                let mins_ago = now.saturating_sub(ts) / 60;

                let label_str = to_label.or_else(|| {
                    state.rules_config.as_ref()?.labels.get(to_addr).map(|s| s.as_str())
                }).map(|l| format!(" ({})", l)).unwrap_or_default();

                ctx.push_str(&format!("  • {:.1} ETH → {}{} [{} min ago]\n", value, to_addr, label_str, mins_ago));
            }
        }
    }

    ctx
}

/// Call OpenClaw /v1/chat/completions synchronously. Returns the agent's answer.
async fn query_openclaw(
    client: &reqwest::Client,
    state: &AppState,
    question: &str,
) -> eyre::Result<String> {
    let token = state.openclaw_gateway_token.as_deref()
        .ok_or_else(|| eyre::eyre!("OpenClaw gateway token not configured"))?;

    let context = build_context(state);

    let body = serde_json::json!({
        "model": "openclaw",
        "messages": [
            {
                "role": "user",
                "content": format!("{context}\nQuestion from a paying agent: {question}")
            }
        ]
    });

    let resp = client
        .post(format!("http://127.0.0.1:{}/v1/chat/completions", state.x402.openclaw_port))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(eyre::eyre!("OpenClaw returned {status}: {text}"));
    }

    let json: serde_json::Value = resp.json().await?;
    let answer = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|item| item.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("No response from agent")
        .to_string();

    Ok(answer)
}
