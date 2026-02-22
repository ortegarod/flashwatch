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

    let state = Arc::new(AppState {
        tx: tx.clone(),
        store,
    });

    // Spawn the upstream flashblocks reader (with optional rule engine)
    let ws_url = ws_url.to_string();
    let reader_state = state.clone();
    let rules_engine = rules_engine.map(|e| Arc::new(e));
    let rules_ref = rules_engine.clone();
    tokio::spawn(async move {
        let mut retry_delay = 2u64;
        loop {
            match upstream_reader(&ws_url, &reader_state.tx, rules_ref.as_ref(), &reader_state.store).await {
                Ok(()) => break,
                Err(e) => {
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
        .route("/alerts/recent", get(recent_alerts_handler));

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

async fn upstream_reader(
    ws_url: &str,
    tx: &broadcast::Sender<String>,
    rules: Option<&Arc<tokio::sync::Mutex<RuleEngine>>>,
    store: &Option<AlertStore>,
) -> eyre::Result<()> {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
    info!("Connected to upstream flashblocks feed");

    let mut current_block: Option<u64> = None;

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
        let _ = tx.send(enriched);

        // Run rule engine if configured
        if let Some(rules_arc) = rules {
            if let Ok(fb) = serde_json::from_str::<crate::types::FlashblockMessage>(&text) {
                let block_number = fb.block_number().or(current_block);
                if fb.block_number().is_some() {
                    current_block = fb.block_number();
                }

                let mut engine = rules_arc.lock().await;
                for tx_val in &fb.diff.transactions {
                    if let Some(tx_hex) = tx_val.as_str() {
                        if let Some(decoded) = crate::decode::decode_raw_tx(tx_hex) {
                            let alerts = engine.check(&decoded, block_number, fb.index);
                            if let Some(store) = store {
                                for alert in &alerts {
                                    if let Err(e) = store.insert(alert) {
                                        tracing::debug!("Failed to store alert: {}", e);
                                    }
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

