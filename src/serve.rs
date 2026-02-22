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
    response::Html,
    routing::get,
};
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

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .route("/alerts", get(alerts_handler))
        .route("/alerts/stats", get(stats_handler))
        .with_state(state);

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

const DASHBOARD_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>flashwatch — Base Flashblocks Dashboard</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { font-family: 'SF Mono','Fira Code','JetBrains Mono',monospace; background: #0a0a0f; color: #e0e0e0; overflow-x: hidden; }
  .header { padding: 20px 32px; border-bottom: 1px solid #1a1a2e; display: flex; align-items: center; gap: 16px; }
  .header h1 { font-size: 20px; font-weight: 600; color: #fff; }
  .header h1 span { color: #fbbf24; }
  .header .sub { font-size: 12px; color: #6b7280; margin-left: auto; }
  .status { font-size: 12px; padding: 4px 10px; border-radius: 12px; background: #1a1a2e; }
  .status.connected { color: #4ade80; border: 1px solid #166534; }
  .status.disconnected { color: #f87171; border: 1px solid #7f1d1d; }
  .metrics { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; padding: 20px 32px; }
  .mc { background: #111118; border: 1px solid #1a1a2e; border-radius: 12px; padding: 14px; }
  .mc .label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.06em; color: #6b7280; margin-bottom: 2px; }
  .mc .val { font-size: 26px; font-weight: 700; color: #fff; }
  .mc .sub { font-size: 11px; color: #6b7280; margin-top: 1px; }
  .val.blue { color: #60a5fa; } .val.green { color: #4ade80; } .val.yellow { color: #fbbf24; } .val.purple { color: #a78bfa; }
  .charts { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; padding: 0 32px 20px; }
  .chart-card { background: #111118; border: 1px solid #1a1a2e; border-radius: 12px; padding: 14px; }
  .chart-card h3 { font-size: 12px; color: #9ca3af; margin-bottom: 10px; }
  canvas { width: 100% !important; height: 140px !important; }
  .panels { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; padding: 0 32px 32px; }
  .panel h3 { font-size: 12px; color: #9ca3af; margin-bottom: 10px; }
  .panel-body { background: #111118; border: 1px solid #1a1a2e; border-radius: 12px; overflow-y: auto; font-size: 13px; }
  .panel-body.short { max-height: 260px; }
  .panel-body.tall { max-height: 500px; }

  /* Left: Block feed */
  .fb-row { padding: 6px 14px; border-bottom: 1px solid #0d0d15; display: flex; gap: 10px; align-items: center; }
  .fb-row .time { color: #4b5563; min-width: 80px; font-size: 11px; }
  .fb-row .idx { color: #fbbf24; min-width: 36px; font-weight: 600; }
  .fb-row .txs { color: #4ade80; min-width: 50px; }
  .fb-row .gas { color: #60a5fa; min-width: 70px; }
  .fb-row .blk { color: #a78bfa; font-weight: 600; }
  .fb-row.new-block { background: rgba(99,102,241,0.06); border-left: 3px solid #6366f1; }

  /* Right: Protocol leaderboard */
  .proto-row { padding: 8px 14px; border-bottom: 1px solid #0d0d15; }
  .proto-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px; }
  .proto-name { font-weight: 600; font-size: 13px; }
  .proto-count { color: #6b7280; font-size: 12px; }
  .proto-bar { height: 6px; border-radius: 3px; background: #1a1a2e; overflow: hidden; }
  .proto-bar-fill { height: 100%; border-radius: 3px; transition: width 0.4s ease; }

  /* Notable events */
  .event-row { padding: 8px 14px; border-bottom: 1px solid #0d0d15; display: flex; gap: 8px; align-items: center; }
  .event-row .emoji { font-size: 15px; min-width: 22px; }
  .event-row .desc { color: #e0e0e0; flex: 1; }
  .event-row .val { color: #4ade80; font-weight: 600; white-space: nowrap; }
  .event-row .ts { color: #4b5563; font-size: 11px; min-width: 70px; text-align: right; }
  .event-row.dex { border-left: 3px solid #22d3ee; }
  .event-row.bridge { border-left: 3px solid #a78bfa; }
  .event-row.lending { border-left: 3px solid #fbbf24; }
  .event-row.whale { border-left: 3px solid #f59e0b; background: rgba(245,158,11,0.04); }
  .event-row.token { border-left: 3px solid #4ade80; }
  .event-row.nft { border-left: 3px solid #f472b6; }

  /* Block summary cards */
  .block-card { padding: 10px 14px; border-bottom: 1px solid #1a1a2e; }
  .block-card .block-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px; }
  .block-card .block-num { color: #60a5fa; font-weight: 700; font-size: 14px; }
  .block-card .block-time { color: #4b5563; font-size: 11px; }
  .block-card .block-stats { display: flex; gap: 14px; font-size: 12px; color: #9ca3af; }
  .block-card .block-stats span { display: flex; align-items: center; gap: 3px; }
  .block-card .block-protos { margin-top: 4px; display: flex; gap: 6px; flex-wrap: wrap; }
  .block-card .proto-tag { font-size: 10px; padding: 2px 8px; border-radius: 8px; background: #1a1a2e; color: #9ca3af; }
  .proto-tag.dex { color: #22d3ee; border: 1px solid #164e63; }
  .proto-tag.bridge { color: #a78bfa; border: 1px solid #4c1d95; }
  .proto-tag.lending { color: #fbbf24; border: 1px solid #713f12; }
  .proto-tag.token { color: #4ade80; border: 1px solid #166534; }

  @media (max-width: 900px) {
    .charts, .panels { grid-template-columns: 1fr; }
    .metrics { grid-template-columns: repeat(3, 1fr); }
  }
</style>
</head>
<body>
<div class="header">
  <h1>⚡ flash<span>watch</span></h1>
  <div class="status disconnected" id="status">Connecting...</div>
  <div class="sub">Base L2 · 200ms flashblocks · live</div>
</div>

<div class="metrics">
  <div class="mc"><div class="label">Block</div><div class="val blue" id="m-block">—</div><div class="sub" id="m-block-sub">waiting</div></div>
  <div class="mc"><div class="label">Flashblocks</div><div class="val yellow" id="m-fb-count">—</div><div class="sub" id="m-fb-rate">—/s</div></div>
  <div class="mc"><div class="label">Transactions</div><div class="val green" id="m-txs">—</div><div class="sub">in current block</div></div>
  <div class="mc"><div class="label">Gas Used</div><div class="val purple" id="m-gas">—</div><div class="sub">cumulative</div></div>
  <div class="mc"><div class="label">Base Fee</div><div class="val" id="m-basefee">—</div><div class="sub">gwei</div></div>
  <div class="mc"><div class="label">Blocks</div><div class="val" id="m-blocks">0</div><div class="sub" id="m-uptime">—</div></div>
</div>

<div class="charts">
  <div class="chart-card"><h3>Transactions per Flashblock</h3><canvas id="chart-txs"></canvas></div>
  <div class="chart-card"><h3>Gas Used per Flashblock</h3><canvas id="chart-gas"></canvas></div>
</div>

<div class="panels">
  <div class="panel">
    <h3>Protocol Activity (rolling 30s)</h3>
    <div class="panel-body short" id="leaderboard"></div>
    <h3 style="margin-top:14px">Recent Blocks</h3>
    <div class="panel-body short" id="block-summary"></div>
  </div>
  <div class="panel">
    <h3>🔔 Notable Events</h3>
    <div class="panel-body tall" id="events"></div>
  </div>
</div>

<script>
const MAX_POINTS=80, MAX_EVENTS=60, MAX_BLOCKS=20;
const PROTO_COLORS={dex:'#22d3ee',bridge:'#a78bfa',lending:'#fbbf24',token:'#4ade80',nft:'#f472b6',system:'#4b5563',unknown:'#6b7280'};
const EMOJI={dex:'🔄',bridge:'🌉',token:'💰',lending:'🏦',nft:'🖼️',system:'⚙️',unknown:'📦'};

let S={currentPayload:null,blockNumber:null,fbCount:0,txsInBlock:0,gasInBlock:0,
  baseFee:null,blocksTotal:0,totalFb:0,startTime:Date.now(),
  txsHistory:[],gasHistory:[],
  protoActivity:{},protoWindow:[],
  currentBlockTxs:0,currentBlockGas:0,currentBlockProtos:{},currentBlockStart:null,
};

function drawChart(id,data,color){
  const c=document.getElementById(id),ctx=c.getContext('2d');
  const dpr=devicePixelRatio||1,r=c.getBoundingClientRect();
  c.width=r.width*dpr;c.height=r.height*dpr;ctx.scale(dpr,dpr);
  const w=r.width,h=r.height;ctx.clearRect(0,0,w,h);
  if(data.length<2)return;
  const max=Math.max(...data,1),step=w/(MAX_POINTS-1);
  const si=Math.max(0,data.length-MAX_POINTS);
  ctx.beginPath();ctx.strokeStyle=color;ctx.lineWidth=2;
  for(let i=si;i<data.length;i++){
    const x=(i-si)*step,y=h-(data[i]/max)*(h-10)-5;
    i===si?ctx.moveTo(x,y):ctx.lineTo(x,y);
  }
  ctx.stroke();
  ctx.lineTo((data.length-1-si)*step,h);ctx.lineTo(0,h);ctx.closePath();
  ctx.fillStyle=color.replace('1)','0.08)');ctx.fill();
  ctx.fillStyle='#4b5563';ctx.font='10px monospace';ctx.fillText(fmtN(max),4,12);
}
function fmtN(n){return n>=1e6?(n/1e6).toFixed(1)+'M':n>=1e3?(n/1e3).toFixed(1)+'K':''+n}
function fmtG(n){return n>=1e6?(n/1e6).toFixed(2)+'M':n>=1e3?(n/1e3).toFixed(1)+'K':''+n}
function ts(){const d=new Date();return d.toTimeString().slice(0,8)+'.'+String(d.getMilliseconds()).padStart(3,'0')}

function updateUI(){
  document.getElementById('m-block').textContent=S.blockNumber??'—';
  document.getElementById('m-fb-count').textContent=S.fbCount;
  document.getElementById('m-txs').textContent=S.txsInBlock;
  document.getElementById('m-gas').textContent=fmtG(S.gasInBlock);
  document.getElementById('m-basefee').textContent=S.baseFee!=null?S.baseFee.toFixed(4):'—';
  document.getElementById('m-blocks').textContent=S.blocksTotal;
  const el=Math.floor((Date.now()-S.startTime)/1000);
  document.getElementById('m-uptime').textContent=Math.floor(el/60)+'m '+el%60+'s';
  if(S.totalFb>0)document.getElementById('m-fb-rate').textContent=(S.totalFb/(el||1)).toFixed(1)+'/s';
  drawChart('chart-txs',S.txsHistory,'rgba(74,222,128,1)');
  drawChart('chart-gas',S.gasHistory,'rgba(96,165,250,1)');
  renderLeaderboard();
}

function renderLeaderboard(){
  // Expire old entries (>30s)
  const now=Date.now();
  S.protoWindow=S.protoWindow.filter(e=>now-e.t<30000);
  // Aggregate
  const counts={};
  for(const e of S.protoWindow){counts[e.n]=(counts[e.n]||0)+1;}
  const sorted=Object.entries(counts).sort((a,b)=>b[1]-a[1]).slice(0,8);
  const maxC=sorted.length?sorted[0][1]:1;
  const lb=document.getElementById('leaderboard');
  lb.innerHTML='';
  if(!sorted.length){lb.innerHTML='<div style="padding:14px;color:#4b5563">Waiting for protocol activity...</div>';return;}
  for(const [name,count] of sorted){
    const cat=S.protoActivity[name]||'unknown';
    const color=PROTO_COLORS[cat]||'#6b7280';
    const pct=Math.round(count/maxC*100);
    const d=document.createElement('div');d.className='proto-row';
    d.innerHTML=`<div class="proto-header"><span class="proto-name" style="color:${color}">${EMOJI[cat]||'📦'} ${name}</span><span class="proto-count">${count} txs</span></div><div class="proto-bar"><div class="proto-bar-fill" style="width:${pct}%;background:${color}"></div></div>`;
    lb.appendChild(d);
  }
}

function sealBlock(){
  if(!S.currentBlockStart)return;
  const bs=document.getElementById('block-summary');
  const d=document.createElement('div');d.className='block-card';
  const protos=Object.entries(S.currentBlockProtos).sort((a,b)=>b[1]-a[1]);
  const protoTags=protos.slice(0,5).map(([n,c])=>{
    const cat=S.protoActivity[n]||'unknown';
    return `<span class="proto-tag ${cat}">${n} ×${c}</span>`;
  }).join('');
  d.innerHTML=`<div class="block-head"><span class="block-num">Block ${S.blockNumber}</span><span class="block-time">${ts()}</span></div><div class="block-stats"><span>📦 ${S.currentBlockTxs} txs</span><span>⛽ ${fmtG(S.currentBlockGas)}</span><span>⚡ ${S.fbCount} fb</span></div>${protoTags?`<div class="block-protos">${protoTags}</div>`:''}`;
  bs.insertBefore(d,bs.firstChild);
  while(bs.children.length>MAX_BLOCKS)bs.removeChild(bs.lastChild);
}

function addEvent(emoji,desc,value,cat){
  const el=document.getElementById('events');
  const d=document.createElement('div');
  d.className='event-row '+(cat||'');
  d.innerHTML=`<span class="emoji">${emoji}</span><span class="desc">${desc}</span>${value?`<span class="val">${value}</span>`:''}<span class="ts">${ts()}</span>`;
  el.insertBefore(d,el.firstChild);
  while(el.children.length>MAX_EVENTS)el.removeChild(el.lastChild);
}

function handleMessage(fb){
  // New block
  if(fb.payload_id!==S.currentPayload){
    if(S.currentPayload)sealBlock();
    S.blocksTotal++;S.currentPayload=fb.payload_id;
    S.fbCount=0;S.txsInBlock=0;S.gasInBlock=0;
    S.currentBlockTxs=0;S.currentBlockGas=0;S.currentBlockProtos={};S.currentBlockStart=Date.now();
  }
  S.fbCount++;S.totalFb++;
  const txC=fb.diff?.transactions?.length??0;
  S.txsInBlock+=txC;S.currentBlockTxs+=txC;
  S.txsHistory.push(txC);if(S.txsHistory.length>MAX_POINTS*2)S.txsHistory=S.txsHistory.slice(-MAX_POINTS);
  const gas=parseInt(fb.diff?.gas_used??'0x0',16);
  S.gasInBlock+=gas;S.currentBlockGas+=gas;
  S.gasHistory.push(gas);if(S.gasHistory.length>MAX_POINTS*2)S.gasHistory=S.gasHistory.slice(-MAX_POINTS);
  if(fb.base?.block_number)S.blockNumber=parseInt(fb.base.block_number,16);
  if(fb.base?.base_fee_per_gas)S.baseFee=parseInt(fb.base.base_fee_per_gas,16)/1e9;

  // Process decoded txs
  const now=Date.now();
  for(const tx of (fb._decoded_txs||[])){
    if(tx.raw)continue;
    const label=tx.to_label;
    if(label){
      const name=label.name;
      const cat=tx.category||'unknown';
      S.protoWindow.push({t:now,n:name});
      S.protoActivity[name]=cat;
      S.currentBlockProtos[name]=(S.currentBlockProtos[name]||0)+1;
    }
    // Notable event thresholds
    const action=tx.action||'';
    const target=label?label.name:(tx.to?tx.to.slice(0,10)+'…':'???');
    const cat=tx.category||'unknown';

    // Big ETH transfers (>0.5 ETH)
    if(tx.value_eth>0.5){
      addEvent('💸',`${action||'Transfer'} → ${target}`,tx.value_eth.toFixed(4)+' ETH',cat);
    }
    // DEX swaps (always notable if labeled)
    else if(cat==='dex'&&action){
      addEvent('🔄',`${action} → ${target}`,'',cat);
    }
    // Bridge activity
    else if(cat==='bridge'){
      addEvent('🌉',`${action||'Bridge'} → ${target}`,tx.value_eth>0.001?tx.value_eth.toFixed(4)+' ETH':'',cat);
    }
    // Lending
    else if(cat==='lending'&&action){
      addEvent('🏦',`${action} → ${target}`,'',cat);
    }
    // NFT
    else if(cat==='nft'&&action){
      addEvent('🖼️',`${action} → ${target}`,'',cat);
    }
  }

  // Whale alerts
  for(const w of (fb._whale_alerts||[])){
    if(parseFloat(w.balance_eth)>5){
      addEvent('🐋',`Whale balance: ${w.address.slice(0,10)}…`,w.balance_eth+' ETH','whale');
    }
  }

  updateUI();
}

function connect(){
  const ws=new WebSocket(`${location.protocol==='https:'?'wss':'ws'}://${location.host}/ws`);
  const st=document.getElementById('status');
  ws.onopen=()=>{st.textContent='Connected';st.className='status connected';};
  ws.onclose=()=>{st.textContent='Reconnecting...';st.className='status disconnected';setTimeout(connect,2000);};
  ws.onerror=()=>ws.close();
  ws.onmessage=e=>{try{handleMessage(JSON.parse(e.data))}catch(err){console.error(err)}};
}
connect();
setInterval(updateUI,1000);
</script>
</body>
</html>
"##;
