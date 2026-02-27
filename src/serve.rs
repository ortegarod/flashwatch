//! Web dashboard server — serves HTML + proxies flashblocks over WebSocket.
//! Also runs the rule engine and stores alerts in SQLite.

use std::collections::HashMap;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    routing::get,
};
use tower_http::services::ServeDir;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tracing::info;

use crate::rules::RuleEngine;
use crate::store::{AlertQuery, AlertStore};

struct AppState {
    tx: broadcast::Sender<String>,
    store: Option<AlertStore>,
    health: tokio::sync::RwLock<HealthInfo>,
    rules_config: Option<crate::rules::RulesConfig>,
    rpc_url: String,
}

#[derive(Default, Clone, serde::Serialize)]
struct HealthInfo {
    connected: bool,
    uptime_secs: u64,
    reconnect_count: u64,
    total_flashblocks: u64,
    total_transactions: u64,
    blocks_seen: u64,
    last_block: Option<u64>,
    last_message_epoch: u64,
    started_epoch: u64,
}

pub async fn run(
    ws_url: &str,
    _rpc_url: &str,
    bind: &str,
    port: u16,
    rules_path: Option<&str>,
    db_path: Option<&str>,
    static_dir: Option<&str>,
) -> eyre::Result<()> {
    let (tx, _) = broadcast::channel::<String>(256);

    // Load rules engine if config provided
    let rules_engine = if let Some(rp) = rules_path {
        let rules_str = std::fs::read_to_string(rp)?;
        let engine = RuleEngine::from_toml(&rules_str)?;
        let rule_count = engine.config.rules.iter().filter(|r| r.enabled).count();
        info!("Loaded {} active alert rules from {}", rule_count, rp);
        Some(tokio::sync::Mutex::new(engine))
    } else {
        None
    };

    // Open SQLite store
    let store = {
        let path = db_path.unwrap_or("flashwatch.db");
        let store = AlertStore::open(&PathBuf::from(path))?;
        info!("Alert store at {}", path);
        Some(store)
    };

    let rules_config = if let Some(ref re) = rules_engine {
        Some(re.lock().await.config.clone())
    } else {
        None
    };

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let state = Arc::new(AppState {
        tx: tx.clone(),
        store,
        health: tokio::sync::RwLock::new(HealthInfo {
            started_epoch: now_epoch,
            ..Default::default()
        }),
        rules_config,
        rpc_url: _rpc_url.to_string(),
    });

    // HTTP client for webhook firing
    let has_webhooks = rules_engine.as_ref()
        .map(|re| re.try_lock().ok()
            .map(|e| e.config.rules.iter().any(|r| r.webhook.is_some()))
            .unwrap_or(false))
        .unwrap_or(false);
    let webhook_client: Option<Arc<reqwest::Client>> = if has_webhooks {
        Some(Arc::new(reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?))
    } else {
        None
    };

    // Spawn the upstream flashblocks reader (with optional rule engine)
    let ws_url = ws_url.to_string();
    let reader_state = state.clone();
    let rules_engine = rules_engine.map(|e| Arc::new(e));
    let rules_ref = rules_engine.clone();
    let webhook_client_ref = webhook_client.clone();
    tokio::spawn(async move {
        let mut retry_delay = 2u64;
        loop {
            {
                let mut h = reader_state.health.write().await;
                h.connected = false;
            }
            match upstream_reader_with_health(&ws_url, &reader_state, rules_ref.as_ref(), webhook_client_ref.as_deref()).await {
                Ok(()) => break,
                Err(e) => {
                    {
                        let mut h = reader_state.health.write().await;
                        h.connected = false;
                        h.reconnect_count += 1;
                    }
                    tracing::error!("Upstream disconnected: {}. Reconnecting in {}s...", e, retry_delay);
                    tokio::time::sleep(std::time::Duration::from_secs(retry_delay)).await;
                    retry_delay = (retry_delay * 2).min(30);
                }
            }
        }
    });

    // Spawn retention pruner (hourly)
    if let Some(ref re) = rules_engine {
        let retention_days = re.lock().await.config.global.retention_days;
        let prune_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                if let Some(ref store) = prune_state.store {
                    match store.prune(retention_days) {
                        Ok(n) if n > 0 => info!("Pruned {} alerts older than {}d", n, retention_days),
                        _ => {}
                    }
                }
            }
        });
    }

    // Resolve static directory: CLI flag > ./static/ > embedded fallback
    let static_path = static_dir
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let p = std::path::PathBuf::from("static");
            if p.is_dir() { Some(p) } else { None }
        });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/alerts", get(alerts_handler))
        .route("/alerts/stats", get(stats_handler))
        .route("/alerts/recent", get(recent_alerts_handler))
        .route("/api/health", get(health_handler))
        .route("/api/rules", get(rules_handler))
        .route("/api/track/{tx_hash}", get(track_handler))
        .route("/api/info", get(info_handler));

    let app = if let Some(ref dir) = static_path {
        info!("Serving frontend from {}", dir.display());
        app.fallback_service(ServeDir::new(dir))
    } else {
        info!("Serving embedded frontend (no static/ directory found)");
        app.route("/", get(index_fallback))
    };

    let app = app.with_state(state);

    let addr: SocketAddr = format!("{}:{}", bind, port).parse()?;
    info!("Dashboard at http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn alerts_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let Some(ref store) = state.store else {
        return Json(serde_json::json!({"error": "no store configured"}));
    };
    let query = AlertQuery::from_params(&params);
    match store.query(&query) {
        Ok(alerts) => Json(serde_json::json!({"alerts": alerts, "count": alerts.len()})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn stats_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let Some(ref store) = state.store else {
        return Json(serde_json::json!({"error": "no store configured"}));
    };
    match store.stats() {
        Ok(stats) => Json(stats),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn recent_alerts_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let Some(ref store) = state.store else {
        return Json(serde_json::json!([]));
    };
    let mut params = std::collections::HashMap::new();
    params.insert("limit".into(), "20".into());
    let query = AlertQuery::from_params(&params);
    match store.query(&query) {
        Ok(alerts) => Json(serde_json::json!(alerts)),
        Err(_) => Json(serde_json::json!([])),
    }
}

async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let h = state.health.read().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Json(serde_json::json!({
        "connected": h.connected,
        "uptime_secs": now.saturating_sub(h.started_epoch),
        "reconnect_count": h.reconnect_count,
        "total_flashblocks": h.total_flashblocks,
        "total_transactions": h.total_transactions,
        "blocks_seen": h.blocks_seen,
        "last_block": h.last_block,
        "last_message_ago_secs": if h.last_message_epoch > 0 { now.saturating_sub(h.last_message_epoch) } else { 0 },
        "started_epoch": h.started_epoch,
    }))
}

async fn rules_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    match &state.rules_config {
        Some(config) => {
            let rules: Vec<serde_json::Value> = config.rules.iter().map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "enabled": r.enabled,
                    "trigger": format!("{:?}", r.trigger),
                    "webhook": r.webhook.is_some(),
                    "cooldown_secs": r.cooldown_secs.unwrap_or(config.global.cooldown_secs),
                })
            }).collect();
            Json(serde_json::json!({
                "rules": rules,
                "global": {
                    "cooldown_secs": config.global.cooldown_secs,
                    "max_per_minute": config.global.max_per_minute,
                    "retention_days": config.global.retention_days,
                }
            }))
        }
        None => Json(serde_json::json!({"rules": [], "global": null})),
    }
}

async fn track_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(tx_hash): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    // Check receipt via RPC
    let client = reqwest::Client::new();
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getTransactionReceipt",
        "params": [tx_hash]
    });
    match client.post(&state.rpc_url).json(&req).send().await {
        Ok(resp) => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(result) = body.get("result") {
                    if result.is_null() {
                        Json(serde_json::json!({"status": "pending", "tx_hash": tx_hash}))
                    } else {
                        let block = result.get("blockNumber").and_then(|b| b.as_str())
                            .and_then(|b| u64::from_str_radix(b.trim_start_matches("0x"), 16).ok());
                        let status = result.get("status").and_then(|s| s.as_str())
                            .map(|s| if s == "0x1" { "success" } else { "failed" });
                        let gas_used = result.get("gasUsed").and_then(|g| g.as_str())
                            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok());
                        Json(serde_json::json!({
                            "status": "confirmed",
                            "tx_hash": tx_hash,
                            "block_number": block,
                            "tx_status": status,
                            "gas_used": gas_used,
                            "receipt": result,
                        }))
                    }
                } else {
                    Json(serde_json::json!({"status": "error", "message": "no result in response"}))
                }
            } else {
                Json(serde_json::json!({"status": "error", "message": "failed to parse response"}))
            }
        }
        Err(e) => Json(serde_json::json!({"status": "error", "message": e.to_string()})),
    }
}

async fn info_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let client = reqwest::Client::new();

    let mut info = serde_json::json!({});

    // Get latest block
    let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":["latest",false]});
    if let Ok(resp) = client.post(&state.rpc_url).json(&req).send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(result) = body.get("result") {
                let block_num = result.get("number").and_then(|n| n.as_str())
                    .and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok());
                let gas_used = result.get("gasUsed").and_then(|g| g.as_str())
                    .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok());
                let base_fee = result.get("baseFeePerGas").and_then(|b| b.as_str())
                    .and_then(|b| u64::from_str_radix(b.trim_start_matches("0x"), 16).ok())
                    .map(|w| w as f64 / 1e9);
                let tx_count = result.get("transactions").and_then(|t| t.as_array()).map(|a| a.len());
                let timestamp = result.get("timestamp").and_then(|t| t.as_str())
                    .and_then(|t| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok());

                info = serde_json::json!({
                    "chain": "Base Mainnet",
                    "chain_id": 8453,
                    "block_number": block_num,
                    "gas_used": gas_used,
                    "base_fee_gwei": base_fee,
                    "tx_count": tx_count,
                    "timestamp": timestamp,
                });
            }
        }
    }

    // Get chain ID
    let req = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"eth_chainId","params":[]});
    if let Ok(resp) = client.post(&state.rpc_url).json(&req).send().await {
        if let Ok(body) = resp.json::<serde_json::Value>().await {
            if let Some(result) = body.get("result").and_then(|r| r.as_str()) {
                if let Ok(id) = u64::from_str_radix(result.trim_start_matches("0x"), 16) {
                    info["chain_id"] = serde_json::json!(id);
                }
            }
        }
    }

    Json(info)
}

async fn index_fallback() -> axum::response::Html<&'static str> {
    axum::response::Html(FALLBACK_HTML)
}

/// Minimal fallback when no static/ directory exists.
const FALLBACK_HTML: &str = r#"<!doctype html><html><head><title>flashwatch</title></head><body style="background:#0a0a0f;color:#e0e0e0;font-family:monospace;padding:40px">
<h1>⚡ flashwatch</h1><p>No <code>static/</code> directory found. Run from the flashwatch repo root or pass <code>--static-dir</code>.</p>
<p>API endpoints available: <a href="/alerts" style="color:#60a5fa">/alerts</a> · <a href="/alerts/stats" style="color:#60a5fa">/alerts/stats</a></p>
</body></html>"#;

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

async fn upstream_reader_with_health(
    ws_url: &str,
    state: &Arc<AppState>,
    rules: Option<&Arc<tokio::sync::Mutex<RuleEngine>>>,
    http_client: Option<&reqwest::Client>,
) -> eyre::Result<()> {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
    info!("Connected to upstream flashblocks feed");

    {
        let mut h = state.health.write().await;
        h.connected = true;
    }

    let mut current_block: Option<u64> = None;
    let mut prev_payload: Option<String> = None;

    while let Some(Ok(msg)) = ws.next().await {
        let data = match msg {
            TungMessage::Text(t) => t.as_bytes().to_vec(),
            TungMessage::Binary(b) => b.to_vec(),
            TungMessage::Ping(_) | TungMessage::Pong(_) => continue,
            TungMessage::Close(_) => break,
            _ => continue,
        };

        let text = match decode_message(&data) {
            Some(t) => t,
            None => continue,
        };

        // Decode transactions and enrich the message
        let enriched = enrich_flashblock(&text);
        let _ = state.tx.send(enriched);

        // Parse for health tracking + rules
        if let Ok(fb) = serde_json::from_str::<crate::types::FlashblockMessage>(&text) {
            let block_number = fb.block_number().or(current_block);
            if fb.block_number().is_some() {
                current_block = fb.block_number();
            }

            let now_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            // Update health
            {
                let mut h = state.health.write().await;
                h.total_flashblocks += 1;
                h.total_transactions += fb.tx_count() as u64;
                h.last_message_epoch = now_epoch;
                h.last_block = block_number;
                if prev_payload.as_ref() != Some(&fb.payload_id) {
                    h.blocks_seen += 1;
                    prev_payload = Some(fb.payload_id.clone());
                }
            }

            // Run rule engine
            if let Some(rules_arc) = rules {
                let mut engine = rules_arc.lock().await;
                for tx_val in &fb.diff.transactions {
                    if let Some(tx_hex) = tx_val.as_str() {
                        if let Some(decoded) = crate::decode::decode_raw_tx(tx_hex) {
                            let alerts = engine.check(&decoded, block_number, fb.index);
                            for alert in &alerts {
                                // Store to SQLite
                                if let Some(ref store) = state.store {
                                    if let Err(e) = store.insert(alert) {
                                        tracing::debug!("Failed to store alert: {}", e);
                                    }
                                }
                                // Fire webhook
                                if let Some(client) = http_client {
                                    crate::alert::fire_webhook_pub(client, &engine.config.rules, alert).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Enrich a flashblock JSON with decoded transaction data.
fn enrich_flashblock(json_str: &str) -> String {
    let mut fb: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return json_str.to_string(),
    };

    let addresses = crate::decode::known_addresses();

    // Decode transactions
    let mut decoded_txs = Vec::new();
    if let Some(txs) = fb.pointer("/diff/transactions").and_then(|t| t.as_array()) {
        for tx_val in txs {
            if let Some(tx_hex) = tx_val.as_str() {
                if let Some(mut dtx) = crate::decode::decode_raw_tx(tx_hex) {
                    // Try to get tx hash from receipts in metadata
                    decoded_txs.push(serde_json::to_value(&dtx).unwrap_or_default());
                } else {
                    decoded_txs.push(serde_json::json!({"raw": &tx_hex[..tx_hex.len().min(40)]}));
                }
            }
        }
    }

    // Also decode account balance changes
    let mut whale_alerts = Vec::new();
    if let Some(balances) = fb.pointer("/metadata/new_account_balances").and_then(|b| b.as_object()) {
        for (addr, val) in balances {
            let addr_lower = addr.to_lowercase();
            if let Some(label) = addresses.get(addr_lower.as_str()) {
                // Skip system addresses
                continue;
            }
            // Check if balance is significant
            if let Some(val_str) = val.as_str() {
                let val_str = val_str.strip_prefix("0x").unwrap_or(val_str);
                if let Ok(wei) = u128::from_str_radix(val_str, 16) {
                    let eth = wei as f64 / 1e18;
                    if eth > 1.0 {
                        whale_alerts.push(serde_json::json!({
                            "address": addr,
                            "balance_eth": format!("{:.4}", eth),
                        }));
                    }
                }
            }
        }
    }

    // Inject decoded data
    fb["_decoded_txs"] = serde_json::Value::Array(decoded_txs);
    if !whale_alerts.is_empty() {
        fb["_whale_alerts"] = serde_json::Value::Array(whale_alerts);
    }

    serde_json::to_string(&fb).unwrap_or_else(|_| json_str.to_string())
}

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

