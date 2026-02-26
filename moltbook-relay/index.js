/**
 * FlashWatch → OpenClaw Relay
 *
 * Receives alert webhooks from flashwatch and forwards them to
 * OpenClaw's /hooks/agent endpoint. The agent (Kyro) interprets
 * the transaction and posts to Moltbook directly.
 *
 * No AI logic here — just payload formatting + forwarding.
 */

const http = require('http');
const fs = require('fs');
const path = require('path');

// ── Config ────────────────────────────────────────────────────────────────────

const PORT = process.env.RELAY_PORT || 4747;
const BIND = process.env.RELAY_BIND || '127.0.0.1';

// ── Credentials ───────────────────────────────────────────────────────────────

const CREDS_PATH = path.join(process.env.HOME, '.config/flashwatch/credentials.json');
let HOOKS_TOKEN = '';
let OPENCLAW_URL = 'http://127.0.0.1:18789';

try {
  const creds = JSON.parse(fs.readFileSync(CREDS_PATH, 'utf8'));
  HOOKS_TOKEN = creds.hooks_token || '';
  OPENCLAW_URL = creds.openclaw_url || OPENCLAW_URL;
} catch (e) {
  console.error('Failed to load credentials:', e.message);
  process.exit(1);
}

if (!HOOKS_TOKEN) {
  console.error('hooks_token missing from credentials');
  process.exit(1);
}

// ── Known Addresses ───────────────────────────────────────────────────────────

const KNOWN_ADDRESSES = {
  '0x71660c4005ba85c37ccec55d0c4493e66fe775d3': 'Coinbase Hot Wallet',
  '0xa9d1e08c7793af67e9d92fe308d5697fb81d3e43': 'Coinbase Cold Storage',
  '0x503828976d22510aad0201ac7ec88293211d23da': 'Coinbase 2',
  '0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740': 'Coinbase 3',
  '0x28c6c06298d514db089934071355e5743bf21d60': 'Binance Hot Wallet',
  '0x21a31ee1afc51d94c2efccaa2092ad1028285549': 'Binance Cold Wallet',
  '0x3154cf16ccdb4c6d922629664174b904d80f2c35': 'Base Bridge (L1)',
  '0x4200000000000000000000000000000000000010': 'Base L2 Bridge',
  '0x2626664c2603336e57b271c5c0b26f421741e481': 'Uniswap V3 Router (Base)',
  '0x198ef1ec325a96cc354c7266a038be8b5c558f67': 'Uniswap Universal Router (Base)',
  '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913': 'USDC (Base)',
};

function label(addr) {
  if (!addr) return null;
  return KNOWN_ADDRESSES[addr.toLowerCase()] || null;
}

function formatAlert(payload) {
  const { rule, alert } = payload;
  const from = alert.from ? `${alert.from}${label(alert.from) ? ` (${label(alert.from)})` : ''}` : 'unknown';
  const to   = alert.to   ? `${alert.to}${label(alert.to)   ? ` (${label(alert.to)})` : ''}` : 'unknown';
  const value = alert.value_eth != null ? `${alert.value_eth} ETH` : (alert.value_usd ? `$${alert.value_usd}` : 'unknown amount');
  const txHash = alert.tx_hash || 'N/A';
  const category = alert.category || rule || 'unknown';

  return [
    `[FlashWatch Alert — Base Mainnet]`,
    `Rule: ${rule}`,
    `Category: ${category}`,
    `From: ${from}`,
    `To: ${to}`,
    `Value: ${value}`,
    `Tx: ${txHash}`,
    alert.block_number ? `Block: ${alert.block_number}` : null,
    ``,
    `You are Kyro, an AI agent monitoring Base blockchain activity in real time.`,
    `Analyze this transaction. Consider: who are these wallets, what does this move signal, is it significant or routine?`,
    `Then write a short, punchy Moltbook post (max 240 chars) with your take and post it to /m/lablab.`,
    `Be direct. Have a personality. No hashtags. No emojis unless they genuinely fit.`,
  ].filter(l => l !== null).join('\n');
}

// ── Forward to OpenClaw ───────────────────────────────────────────────────────

function forwardToOpenClaw(message) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({
      message,
      name: 'FlashWatch',
      deliver: false,        // don't echo to Matrix
      wakeMode: 'now',
    });

    const url = new URL('/hooks/agent', OPENCLAW_URL);
    const options = {
      hostname: url.hostname,
      port: url.port,
      path: url.pathname,
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${HOOKS_TOKEN}`,
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(body),
      },
    };

    const req = http.request(options, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => resolve({ status: res.statusCode, body: data }));
    });

    req.on('error', reject);
    req.write(body);
    req.end();
  });
}

// ── HTTP Server ───────────────────────────────────────────────────────────────

const server = http.createServer(async (req, res) => {
  if (req.method === 'GET' && req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ status: 'ok', service: 'flashwatch-openclaw-relay' }));
    return;
  }

  if (req.method === 'POST' && req.url === '/webhook') {
    let body = '';
    req.on('data', chunk => body += chunk);
    req.on('end', async () => {
      let payload;
      try {
        payload = JSON.parse(body);
      } catch (e) {
        res.writeHead(400);
        res.end('bad json');
        return;
      }

      const message = formatAlert(payload);
      console.log(`[${new Date().toISOString()}] Alert: ${payload.rule} — forwarding to OpenClaw`);

      try {
        const result = await forwardToOpenClaw(message);
        console.log(`[openclaw] ${result.status} — ${result.body.slice(0, 120)}`);
        res.writeHead(result.status < 500 ? 200 : 502);
        res.end('ok');
      } catch (e) {
        console.error('[openclaw] forward failed:', e.message);
        res.writeHead(502);
        res.end('forward failed');
      }
    });
    return;
  }

  res.writeHead(404);
  res.end('not found');
});

server.listen(PORT, BIND, () => {
  console.log(`FlashWatch → OpenClaw relay on http://${BIND}:${PORT}`);
  console.log(`  Webhook:  POST /webhook`);
  console.log(`  Health:   GET  /health`);
  console.log(`  Target:   ${OPENCLAW_URL}/hooks/agent`);
  console.log(`  Token:    ✓ loaded`);
});
