//! Web dashboard server — serves HTML + proxies flashblocks over WebSocket.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Html,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tracing::info;

struct AppState {
    tx: broadcast::Sender<String>,
}

pub async fn run(ws_url: &str, _rpc_url: &str, bind: &str, port: u16) -> eyre::Result<()> {
    let (tx, _) = broadcast::channel::<String>(256);
    let state = Arc::new(AppState { tx: tx.clone() });

    // Spawn the upstream flashblocks reader
    let ws_url = ws_url.to_string();
    tokio::spawn(async move {
        loop {
            if let Err(e) = upstream_reader(&ws_url, &tx).await {
                tracing::error!("Upstream disconnected: {}. Reconnecting in 2s...", e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", bind, port).parse()?;
    info!("Dashboard at http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

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
) -> eyre::Result<()> {
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_url).await?;
    info!("Connected to upstream flashblocks feed");

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

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>flashwatch — Base Flashblocks Dashboard</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', monospace;
    background: #0a0a0f;
    color: #e0e0e0;
    overflow-x: hidden;
  }
  .header {
    padding: 20px 32px;
    border-bottom: 1px solid #1a1a2e;
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .header h1 {
    font-size: 20px;
    font-weight: 600;
    color: #fff;
  }
  .header h1 span { color: #fbbf24; }
  .status {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 12px;
    background: #1a1a2e;
  }
  .status.connected { color: #4ade80; border: 1px solid #166534; }
  .status.disconnected { color: #f87171; border: 1px solid #7f1d1d; }

  .metrics {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 16px;
    padding: 24px 32px;
  }
  .metric-card {
    background: #111118;
    border: 1px solid #1a1a2e;
    border-radius: 12px;
    padding: 16px;
  }
  .metric-card .label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #6b7280;
    margin-bottom: 4px;
  }
  .metric-card .value {
    font-size: 28px;
    font-weight: 700;
    color: #fff;
  }
  .metric-card .sub {
    font-size: 12px;
    color: #6b7280;
    margin-top: 2px;
  }
  .value.blue { color: #60a5fa; }
  .value.green { color: #4ade80; }
  .value.yellow { color: #fbbf24; }
  .value.purple { color: #a78bfa; }

  .charts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    padding: 0 32px 24px;
  }
  .chart-card {
    background: #111118;
    border: 1px solid #1a1a2e;
    border-radius: 12px;
    padding: 16px;
  }
  .chart-card h3 {
    font-size: 13px;
    color: #9ca3af;
    margin-bottom: 12px;
  }
  canvas {
    width: 100% !important;
    height: 160px !important;
  }

  .panels {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    padding: 0 32px 32px;
  }
  .feed, .activity {
  }
  .feed h3, .activity h3 {
    font-size: 13px;
    color: #9ca3af;
    margin-bottom: 12px;
  }
  .feed-list {
    background: #111118;
    border: 1px solid #1a1a2e;
    border-radius: 12px;
    max-height: 340px;
    overflow-y: auto;
    font-size: 13px;
  }
  .feed-item {
    padding: 8px 16px;
    border-bottom: 1px solid #1a1a2e;
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .feed-item:last-child { border-bottom: none; }
  .feed-item .time { color: #6b7280; min-width: 90px; }
  .feed-item .idx { color: #fbbf24; min-width: 40px; font-weight: 600; }
  .feed-item .txs { color: #4ade80; min-width: 60px; }
  .feed-item .gas { color: #60a5fa; min-width: 80px; }
  .feed-item .block { color: #a78bfa; }
  .feed-item.new-block {
    background: rgba(99, 102, 241, 0.08);
    border-left: 3px solid #6366f1;
  }

  .act-item {
    padding: 8px 16px;
    border-bottom: 1px solid #1a1a2e;
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  .act-item:last-child { border-bottom: none; }
  .act-item .emoji { font-size: 16px; min-width: 24px; }
  .act-item .action { color: #e0e0e0; font-weight: 600; }
  .act-item .target { color: #60a5fa; }
  .act-item .val { color: #4ade80; }
  .act-item .addr { color: #6b7280; font-size: 11px; }
  .act-item.whale {
    background: rgba(251, 191, 36, 0.08);
    border-left: 3px solid #fbbf24;
  }
  .act-item.dex { border-left: 3px solid #22d3ee; }
  .act-item.bridge { border-left: 3px solid #a78bfa; }
  .act-item.lending { border-left: 3px solid #fbbf24; }
  .act-item.nft { border-left: 3px solid #f472b6; }

  @media (max-width: 768px) {
    .charts { grid-template-columns: 1fr; }
    .metrics { grid-template-columns: repeat(2, 1fr); }
    .panels { grid-template-columns: 1fr; }
  }
</style>
</head>
<body>

<div class="header">
  <h1>⚡ flash<span>watch</span></h1>
  <div class="status disconnected" id="status">Connecting...</div>
</div>

<div class="metrics">
  <div class="metric-card">
    <div class="label">Block</div>
    <div class="value blue" id="m-block">—</div>
    <div class="sub" id="m-block-sub">waiting for data</div>
  </div>
  <div class="metric-card">
    <div class="label">Flashblocks / block</div>
    <div class="value yellow" id="m-fb-count">—</div>
    <div class="sub" id="m-fb-rate">—/s</div>
  </div>
  <div class="metric-card">
    <div class="label">Transactions</div>
    <div class="value green" id="m-txs">—</div>
    <div class="sub" id="m-txs-sub">in current block</div>
  </div>
  <div class="metric-card">
    <div class="label">Gas Used</div>
    <div class="value purple" id="m-gas">—</div>
    <div class="sub" id="m-gas-sub">cumulative</div>
  </div>
  <div class="metric-card">
    <div class="label">Base Fee</div>
    <div class="value" id="m-basefee">—</div>
    <div class="sub">gwei</div>
  </div>
  <div class="metric-card">
    <div class="label">Blocks Seen</div>
    <div class="value" id="m-blocks">0</div>
    <div class="sub" id="m-uptime">—</div>
  </div>
</div>

<div class="charts">
  <div class="chart-card">
    <h3>Transactions per Flashblock</h3>
    <canvas id="chart-txs"></canvas>
  </div>
  <div class="chart-card">
    <h3>Gas Used (cumulative within block)</h3>
    <canvas id="chart-gas"></canvas>
  </div>
</div>

<div class="panels">
  <div class="feed">
    <h3>Live Feed</h3>
    <div class="feed-list" id="feed"></div>
  </div>
  <div class="activity">
    <h3>⚡ Activity — Decoded Transactions</h3>
    <div class="feed-list" id="activity"></div>
  </div>
</div>

<script>
const MAX_POINTS = 80;
const MAX_FEED = 100;

let state = {
  currentPayload: null,
  blockNumber: null,
  fbCount: 0,
  txsInBlock: 0,
  gasInBlock: 0,
  baseFee: null,
  blocksTotal: 0,
  totalFb: 0,
  startTime: Date.now(),
  txsHistory: [],
  gasHistory: [],
};

// Simple canvas chart
function drawChart(canvasId, data, color, filled = true) {
  const canvas = document.getElementById(canvasId);
  const ctx = canvas.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = rect.width * dpr;
  canvas.height = rect.height * dpr;
  ctx.scale(dpr, dpr);
  const w = rect.width, h = rect.height;

  ctx.clearRect(0, 0, w, h);
  if (data.length < 2) return;

  const max = Math.max(...data, 1);
  const step = w / (MAX_POINTS - 1);

  ctx.beginPath();
  ctx.strokeStyle = color;
  ctx.lineWidth = 2;

  const startIdx = Math.max(0, data.length - MAX_POINTS);
  for (let i = startIdx; i < data.length; i++) {
    const x = (i - startIdx) * step;
    const y = h - (data[i] / max) * (h - 10) - 5;
    if (i === startIdx) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();

  if (filled) {
    ctx.lineTo((data.length - 1 - startIdx) * step, h);
    ctx.lineTo(0, h);
    ctx.closePath();
    ctx.fillStyle = color.replace('1)', '0.1)');
    ctx.fill();
  }

  // Max label
  ctx.fillStyle = '#6b7280';
  ctx.font = '10px monospace';
  ctx.fillText(formatNum(max), 4, 12);
}

function formatNum(n) {
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return n.toString();
}

function formatGas(n) {
  if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return n.toString();
}

function updateUI() {
  document.getElementById('m-block').textContent = state.blockNumber ?? '—';
  document.getElementById('m-fb-count').textContent = state.fbCount;
  document.getElementById('m-txs').textContent = state.txsInBlock;
  document.getElementById('m-gas').textContent = formatGas(state.gasInBlock);
  document.getElementById('m-basefee').textContent =
    state.baseFee != null ? state.baseFee.toFixed(4) : '—';
  document.getElementById('m-blocks').textContent = state.blocksTotal;

  const elapsed = Math.floor((Date.now() - state.startTime) / 1000);
  const min = Math.floor(elapsed / 60);
  const sec = elapsed % 60;
  document.getElementById('m-uptime').textContent =
    `${min}m ${sec}s uptime`;

  if (state.totalFb > 0) {
    const rate = (state.totalFb / (elapsed || 1)).toFixed(1);
    document.getElementById('m-fb-rate').textContent = rate + '/s';
  }

  drawChart('chart-txs', state.txsHistory, 'rgba(74, 222, 128, 1)');
  drawChart('chart-gas', state.gasHistory, 'rgba(96, 165, 250, 1)');
}

function addFeedItem(fb) {
  const feed = document.getElementById('feed');
  const div = document.createElement('div');
  div.className = 'feed-item' + (fb.index === 0 ? ' new-block' : '');

  const now = new Date();
  const time = now.toTimeString().slice(0, 8) + '.' + String(now.getMilliseconds()).padStart(3, '0');
  const txCount = fb.diff?.transactions?.length ?? 0;
  const gasHex = fb.diff?.gas_used ?? '0x0';
  const gas = parseInt(gasHex, 16);
  const blockNum = fb.base?.block_number ? parseInt(fb.base.block_number, 16) : state.blockNumber;

  div.innerHTML = `
    <span class="time">${time}</span>
    <span class="idx">fb${fb.index}</span>
    <span class="txs">${txCount} txs</span>
    <span class="gas">${formatGas(gas)} gas</span>
    ${fb.index === 0 ? `<span class="block">▸ block ${blockNum}</span>` : ''}
  `;

  feed.insertBefore(div, feed.firstChild);
  while (feed.children.length > MAX_FEED) {
    feed.removeChild(feed.lastChild);
  }
}

function handleMessage(fb) {
  // New block?
  if (fb.payload_id !== state.currentPayload) {
    if (state.currentPayload) state.blocksTotal++;
    state.currentPayload = fb.payload_id;
    state.fbCount = 0;
    state.txsInBlock = 0;
    state.gasInBlock = 0;
  }

  state.fbCount++;
  state.totalFb++;

  const txCount = fb.diff?.transactions?.length ?? 0;
  state.txsInBlock += txCount;
  state.txsHistory.push(txCount);
  if (state.txsHistory.length > MAX_POINTS * 2) {
    state.txsHistory = state.txsHistory.slice(-MAX_POINTS);
  }

  const gas = parseInt(fb.diff?.gas_used ?? '0x0', 16);
  state.gasInBlock += gas;
  state.gasHistory.push(gas);
  if (state.gasHistory.length > MAX_POINTS * 2) {
    state.gasHistory = state.gasHistory.slice(-MAX_POINTS);
  }

  if (fb.base?.block_number) {
    state.blockNumber = parseInt(fb.base.block_number, 16);
  }
  if (fb.base?.base_fee_per_gas) {
    state.baseFee = parseInt(fb.base.base_fee_per_gas, 16) / 1e9;
  }

  addFeedItem(fb);
  addDecodedTxs(fb);
  updateUI();
}

function addDecodedTxs(fb) {
  const activity = document.getElementById('activity');
  const txs = fb._decoded_txs || [];

  for (const tx of txs) {
    if (tx.raw) continue; // skip unparsed
    if (!tx.action && tx.value_eth < 0.01) continue; // skip boring txs

    const div = document.createElement('div');
    const cat = tx.category || 'unknown';
    div.className = 'act-item ' + cat;

    const emoji = {dex:'🔄', bridge:'🌉', token:'💰', lending:'🏦', nft:'🖼️', system:'⚙️', unknown:'📦'}[cat] || '📦';
    const target = tx.to_label ? tx.to_label.name : (tx.to ? tx.to.slice(0, 10) + '…' : '???');
    const action = tx.action || (tx.value_eth > 0 ? 'ETH transfer' : 'call');
    const value = tx.value_eth > 0.001 ? `${tx.value_eth.toFixed(4)} ETH` : '';

    div.innerHTML = `
      <span class="emoji">${emoji}</span>
      <span class="action">${action}</span>
      <span class="target">→ ${target}</span>
      ${value ? `<span class="val">${value}</span>` : ''}
    `;

    activity.insertBefore(div, activity.firstChild);
    while (activity.children.length > MAX_FEED) {
      activity.removeChild(activity.lastChild);
    }
  }

  // Whale alerts
  const whales = fb._whale_alerts || [];
  for (const w of whales) {
    const div = document.createElement('div');
    div.className = 'act-item whale';
    div.innerHTML = `
      <span class="emoji">🐋</span>
      <span class="action">Balance change</span>
      <span class="addr">${w.address.slice(0, 10)}…</span>
      <span class="val">${w.balance_eth} ETH</span>
    `;
    activity.insertBefore(div, activity.firstChild);
  }
}

// WebSocket connection
function connect() {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/ws`);
  const statusEl = document.getElementById('status');

  ws.onopen = () => {
    statusEl.textContent = 'Connected';
    statusEl.className = 'status connected';
  };

  ws.onclose = () => {
    statusEl.textContent = 'Disconnected — reconnecting...';
    statusEl.className = 'status disconnected';
    setTimeout(connect, 2000);
  };

  ws.onerror = () => ws.close();

  ws.onmessage = (e) => {
    try {
      const fb = JSON.parse(e.data);
      handleMessage(fb);
    } catch (err) {
      console.error('Parse error:', err);
    }
  };
}

connect();
setInterval(updateUI, 1000);
</script>
</body>
</html>
"##;
