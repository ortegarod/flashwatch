/**
 * FlashWatch → Moltbook Relay
 * Receives alert webhooks from flashwatch, enriches with on-chain context,
 * and posts to Moltbook /m/lablab.
 *
 * Small alerts (<AI_THRESHOLD_ETH): template post, instant, free
 * Big alerts  (≥AI_THRESHOLD_ETH): RPC enrichment + Claude interpretation → rich post
 */

const http = require('http');
const https = require('https');
const fs = require('fs');
const path = require('path');

// ── Config ────────────────────────────────────────────────────────────────────

const PORT = process.env.RELAY_PORT || 4747;
const BIND = process.env.RELAY_BIND || '127.0.0.1';
const AI_THRESHOLD_ETH = parseFloat(process.env.AI_THRESHOLD_ETH || '50');
const BASE_RPC_URL = process.env.BASE_RPC_URL || 'https://mainnet.base.org';
const COOLDOWN_MS = 10 * 60 * 1000; // 10 min between same-rule posts

// ── Credentials ───────────────────────────────────────────────────────────────

const MOLTBOOK_CREDS_PATH = path.join(process.env.HOME, '.config/moltbook/credentials.json');
let MOLTBOOK_API_KEY = '';
try {
  const creds = JSON.parse(fs.readFileSync(MOLTBOOK_CREDS_PATH, 'utf8'));
  MOLTBOOK_API_KEY = creds.api_key || '';
} catch (e) {
  console.error('Failed to load Moltbook credentials:', e.message);
  process.exit(1);
}

// Load .env from repo root for ANTHROPIC_API_KEY if not already set
if (!process.env.ANTHROPIC_API_KEY) {
  const envPath = path.join(__dirname, '..', '.env');
  if (fs.existsSync(envPath)) {
    const lines = fs.readFileSync(envPath, 'utf8').split('\n');
    for (const line of lines) {
      const [k, ...rest] = line.split('=');
      if (k && rest.length) process.env[k.trim()] = rest.join('=').trim();
    }
  }
}
const ANTHROPIC_API_KEY = process.env.ANTHROPIC_API_KEY || '';
if (!ANTHROPIC_API_KEY) {
  console.warn('[warn] ANTHROPIC_API_KEY not set — big alerts will fall back to template posts');
}

// ── Known Addresses ───────────────────────────────────────────────────────────
// Common Base/Ethereum addresses. Match against from/to for labeling.

const KNOWN_ADDRESSES = {
  // Coinbase
  '0x71660c4005ba85c37ccec55d0c4493e66fe775d3': 'Coinbase Hot Wallet',
  '0xa9d1e08c7793af67e9d92fe308d5697fb81d3e43': 'Coinbase Cold Storage',
  '0x503828976d22510aad0201ac7ec88293211d23da': 'Coinbase 2',
  '0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740': 'Coinbase 3',
  // Binance
  '0x28c6c06298d514db089934071355e5743bf21d60': 'Binance Hot Wallet',
  '0x21a31ee1afc51d94c2efccaa2092ad1028285549': 'Binance Cold Wallet',
  // Base Bridge
  '0x3154cf16ccdb4c6d922629664174b904d80f2c35': 'Base Bridge (L1)',
  '0x4200000000000000000000000000000000000010': 'Base L2 Bridge',
  // Uniswap
  '0x2626664c2603336e57b271c5c0b26f421741e481': 'Uniswap V3 Router (Base)',
  '0x198ef1ec325a96cc354c7266a038be8b5c558f67': 'Uniswap Universal Router (Base)',
  // USDC
  '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913': 'USDC (Base)',
};

function labelAddress(addr) {
  if (!addr) return null;
  return KNOWN_ADDRESSES[addr.toLowerCase()] || null;
}

// ── RPC Helpers ───────────────────────────────────────────────────────────────

function rpcCall(method, params) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({ jsonrpc: '2.0', id: 1, method, params });
    const url = new URL(BASE_RPC_URL);
    const options = {
      hostname: url.hostname,
      port: url.port || 443,
      path: url.pathname,
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) },
    };
    const req = (url.protocol === 'https:' ? https : http).request(options, (res) => {
      let data = '';
      res.on('data', c => data += c);
      res.on('end', () => {
        try { resolve(JSON.parse(data).result); }
        catch (e) { reject(new Error('RPC parse error: ' + data.slice(0, 100))); }
      });
    });
    req.on('error', reject);
    req.setTimeout(5000, () => { req.destroy(); reject(new Error('RPC timeout')); });
    req.write(body);
    req.end();
  });
}

function hexToEth(hex) {
  if (!hex || hex === '0x0') return 0;
  return parseInt(hex, 16) / 1e18;
}

async function enrichAddress(addr) {
  if (!addr) return {};
  const lower = addr.toLowerCase();
  const knownLabel = labelAddress(lower);

  try {
    const [txCountHex, balanceHex] = await Promise.all([
      rpcCall('eth_getTransactionCount', [addr, 'latest']),
      rpcCall('eth_getBalance', [addr, 'latest']),
    ]);
    return {
      label: knownLabel,
      txCount: txCountHex ? parseInt(txCountHex, 16) : null,
      balanceEth: balanceHex ? hexToEth(balanceHex) : null,
      isKnown: !!knownLabel,
    };
  } catch (e) {
    console.warn(`[enrich] RPC failed for ${addr}: ${e.message}`);
    return { label: knownLabel, isKnown: !!knownLabel };
  }
}

// ── ENS Lookup ────────────────────────────────────────────────────────────────

function ensLookup(addr) {
  return new Promise((resolve) => {
    const options = {
      hostname: 'api.ensideas.com',
      path: `/ens/resolve/${addr}`,
      method: 'GET',
      headers: { 'Accept': 'application/json' },
    };
    const req = https.request(options, (res) => {
      let data = '';
      res.on('data', c => data += c);
      res.on('end', () => {
        try {
          const json = JSON.parse(data);
          resolve(json.name || null);
        } catch { resolve(null); }
      });
    });
    req.on('error', () => resolve(null));
    req.setTimeout(4000, () => { req.destroy(); resolve(null); });
    req.end();
  });
}

// ── Claude API ────────────────────────────────────────────────────────────────

function callClaude(systemPrompt, userMessage) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({
      model: 'claude-haiku-4-5',
      max_tokens: 300,
      system: systemPrompt,
      messages: [{ role: 'user', content: userMessage }],
    });

    const options = {
      hostname: 'api.anthropic.com',
      path: '/v1/messages',
      method: 'POST',
      headers: {
        'x-api-key': ANTHROPIC_API_KEY,
        'anthropic-version': '2023-06-01',
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(body),
      },
    };

    const req = https.request(options, (res) => {
      let data = '';
      res.on('data', c => data += c);
      res.on('end', () => {
        try {
          const json = JSON.parse(data);
          resolve(json.content?.[0]?.text || null);
        } catch (e) { reject(new Error('Claude parse error')); }
      });
    });
    req.on('error', reject);
    req.setTimeout(15000, () => { req.destroy(); reject(new Error('Claude timeout')); });
    req.write(body);
    req.end();
  });
}

const SYSTEM_PROMPT = `You are FlashWatch, an AI agent monitoring Base L2 flash blocks in real time. You have a sharp, informed personality — like a seasoned on-chain analyst who's seen everything. You're direct, occasionally dry, and you cut through noise.

When a large on-chain alert fires, you investigate the context and post to Moltbook — a social network for AI agents. Your posts are short (2–5 sentences max), readable, and actually say something. No fluff. No generic alerts.

Rules:
- If the address is a known exchange/protocol (Coinbase, Binance, bridge), call it out immediately. These are usually boring.
- If the address is unknown, dormant, or unusual — flag it as worth watching.
- Include the actual numbers (ETH amount, tx count, balance if notable).
- End with a brief take: what does this mean? Should people pay attention?
- Use 1–2 emojis max. Don't overdo it.
- Never say "I detected" or "FlashWatch detected" — write in first person as if you're the analyst.
- Keep it under 280 characters when possible, but don't sacrifice substance for brevity.
- Include "[FlashWatch](https://github.com/ortegarod/flashwatch)" as a footer link.`;

async function generateAIPost(alert, enriched) {
  const { from, to, value_eth, action, category } = alert.tx;
  const fromInfo = enriched.from;
  const toInfo = enriched.to;

  const context = [
    `Alert type: ${alert.rule_name}`,
    `Amount: ${value_eth?.toFixed(4)} ETH`,
    `Action: ${action || category || 'transfer'}`,
    `From: ${fromInfo?.label || enriched.fromEns || from}`,
    fromInfo?.txCount != null ? `  → ${fromInfo.txCount.toLocaleString()} lifetime txs, ${fromInfo.balanceEth?.toFixed(2)} ETH balance` : '',
    `To: ${toInfo?.label || enriched.toEns || to}`,
    toInfo?.txCount != null ? `  → ${toInfo.txCount.toLocaleString()} lifetime txs, ${toInfo.balanceEth?.toFixed(2)} ETH balance` : '',
    `Block: ${alert.block_number} (pre-confirmation flashblock)`,
    fromInfo?.isKnown ? `From is a known entity (${fromInfo.label}).` : 'From address is NOT a known entity.',
    toInfo?.isKnown ? `To is a known entity (${toInfo.label}).` : 'To address is NOT a known entity.',
  ].filter(Boolean).join('\n');

  const userMessage = `Write a Moltbook post for this on-chain alert:\n\n${context}`;

  try {
    const text = await callClaude(SYSTEM_PROMPT, userMessage);
    return text;
  } catch (e) {
    console.error('[claude] Failed:', e.message);
    return null; // will fall back to template
  }
}

// ── Template Post (small alerts / AI fallback) ────────────────────────────────

function formatTemplate(alert) {
  const { rule_name, block_number, tx } = alert;
  const value = tx.value_eth > 0 ? `${tx.value_eth.toFixed(4)} ETH` : '';
  const target = tx.to_label || (tx.to ? `${tx.to.slice(0, 6)}...${tx.to.slice(-4)}` : 'unknown');
  const action = tx.action || tx.category || 'transfer';

  const emojiMap = {
    'whale-transfer': '🐋',
    'large-value': '💰',
    'dex-swap': '🔄',
    'bridge-activity': '🌉',
  };
  const emoji = emojiMap[rule_name] || '🚨';

  return [
    `${emoji} **${rule_name.replace(/-/g, ' ').toUpperCase()}** on Base`,
    ``,
    `• ${action} — ${value} → ${target}`,
    `• Block ${block_number} (pre-confirmation flashblock)`,
    ``,
    `[FlashWatch](https://github.com/ortegarod/flashwatch) — real-time Base monitoring`,
  ].join('\n');
}

// ── Moltbook Post ─────────────────────────────────────────────────────────────

const lastPosted = {};

async function handleAlert(alert) {
  const now = Date.now();
  const lastTime = lastPosted[alert.rule_name] || 0;
  if (now - lastTime < COOLDOWN_MS) {
    console.log(`[cooldown] Skipping ${alert.rule_name} — posted ${Math.round((now - lastTime) / 1000)}s ago`);
    return;
  }

  const valueEth = alert.tx?.value_eth || 0;
  const isLargeAlert = valueEth >= AI_THRESHOLD_ETH && !!ANTHROPIC_API_KEY;

  let content;
  let postType;

  if (isLargeAlert) {
    console.log(`[ai] Large alert (${valueEth.toFixed(2)} ETH ≥ ${AI_THRESHOLD_ETH} ETH threshold) — enriching...`);
    try {
      // Parallel: enrich both addresses + ENS lookups
      const [fromInfo, toInfo, fromEns, toEns] = await Promise.all([
        enrichAddress(alert.tx?.from),
        enrichAddress(alert.tx?.to),
        alert.tx?.from ? ensLookup(alert.tx.from) : Promise.resolve(null),
        alert.tx?.to ? ensLookup(alert.tx.to) : Promise.resolve(null),
      ]);

      console.log(`[enrich] from=${fromInfo?.label || fromEns || 'unknown'}, to=${toInfo?.label || toEns || 'unknown'}`);

      const aiPost = await generateAIPost(alert, { from: fromInfo, to: toInfo, fromEns, toEns });
      if (aiPost) {
        content = aiPost;
        postType = 'ai-interpreted';
      } else {
        content = formatTemplate(alert);
        postType = 'template (ai-fallback)';
      }
    } catch (e) {
      console.error('[enrich] Failed:', e.message);
      content = formatTemplate(alert);
      postType = 'template (enrich-fallback)';
    }
  } else {
    content = formatTemplate(alert);
    postType = isLargeAlert ? 'template (no api key)' : 'template';
  }

  const title = (() => {
    const val = valueEth > 0 ? ` — ${valueEth.toFixed(2)} ETH` : '';
    const label = alert.tx?.to_label ? ` → ${alert.tx.to_label}` : '';
    return `${alert.rule_name.replace(/-/g, ' ')}${val}${label} on Base`;
  })();

  console.log(`[post] type=${postType}, title="${title}"`);

  const body = JSON.stringify({ submolt_name: 'lablab', title, content });
  const options = {
    hostname: 'www.moltbook.com',
    path: '/api/v1/posts',
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${MOLTBOOK_API_KEY}`,
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body),
    },
  };

  const req = https.request(options, (res) => {
    let data = '';
    res.on('data', chunk => data += chunk);
    res.on('end', () => {
      if (res.statusCode === 200 || res.statusCode === 201) {
        lastPosted[alert.rule_name] = now;
        console.log(`[moltbook] ✓ Posted (${postType}): ${title}`);
      } else {
        console.error(`[moltbook] ✗ Error ${res.statusCode}: ${data.slice(0, 300)}`);
      }
    });
  });
  req.on('error', e => console.error('[moltbook] Request failed:', e.message));
  req.write(body);
  req.end();
}

// ── Server ────────────────────────────────────────────────────────────────────

const server = http.createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/webhook') {
    let body = '';
    req.on('data', chunk => body += chunk);
    req.on('end', async () => {
      try {
        const alert = JSON.parse(body);
        console.log(`[alert] ${alert.rule_name} — ${alert.tx?.value_eth?.toFixed(4)} ETH → ${alert.tx?.to_label || alert.tx?.to}`);
        handleAlert(alert); // async, don't await — respond immediately
        res.writeHead(200);
        res.end('ok');
      } catch (e) {
        console.error('[parse error]', e.message);
        res.writeHead(400);
        res.end('bad request');
      }
    });
  } else if (req.method === 'GET' && req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({
      status: 'ok',
      service: 'flashwatch-moltbook-relay',
      aiEnabled: !!ANTHROPIC_API_KEY,
      aiThresholdEth: AI_THRESHOLD_ETH,
      cooldowns: lastPosted,
    }));
  } else {
    res.writeHead(404);
    res.end('not found');
  }
});

server.listen(PORT, BIND, () => {
  console.log(`\nFlashWatch → Moltbook relay on http://${BIND}:${PORT}`);
  console.log(`  Webhook:       POST /webhook`);
  console.log(`  Health:        GET  /health`);
  console.log(`  Moltbook key:  ${MOLTBOOK_API_KEY ? '✓ loaded' : '✗ MISSING'}`);
  console.log(`  AI interpret:  ${ANTHROPIC_API_KEY ? `✓ enabled (threshold: ≥${AI_THRESHOLD_ETH} ETH)` : '✗ disabled (set ANTHROPIC_API_KEY)'}`);
  console.log('');
});
