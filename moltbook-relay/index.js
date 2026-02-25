/**
 * FlashWatch → Moltbook Relay
 * Receives alert webhooks from flashwatch and posts to Moltbook /m/lablab
 */

const http = require('http');
const https = require('https');
const fs = require('fs');
const path = require('path');

const PORT = process.env.RELAY_PORT || 4747;
const BIND = process.env.RELAY_BIND || '127.0.0.1'; // default local; set RELAY_BIND=<tailscale-ip> in production

// Load Moltbook API key
const CREDENTIALS_PATH = path.join(process.env.HOME, '.config/moltbook/credentials.json');
let MOLTBOOK_API_KEY = '';
try {
  const creds = JSON.parse(fs.readFileSync(CREDENTIALS_PATH, 'utf8'));
  MOLTBOOK_API_KEY = creds.api_key || '';
} catch (e) {
  console.error('Failed to load Moltbook credentials:', e.message);
  process.exit(1);
}

// Cooldown: don't post same rule more than once per N minutes
const COOLDOWN_MS = 10 * 60 * 1000; // 10 minutes
const lastPosted = {};

function formatAlert(alert) {
  const { rule_name, block_number, tx } = alert;
  const value = tx.value_eth > 0 ? `${tx.value_eth.toFixed(4)} ETH` : '';
  const target = tx.to_label || (tx.to ? `${tx.to.slice(0, 6)}...${tx.to.slice(-4)}` : 'unknown');
  const action = tx.action || tx.category;

  const emojiMap = {
    'whale-transfer': '🐋',
    'large-value': '💰',
    'dex-swap': '🔄',
    'bridge-activity': '🌉',
  };
  const emoji = emojiMap[rule_name] || '🚨';

  const lines = [
    `${emoji} **${rule_name.replace(/-/g, ' ').toUpperCase()}** detected on Base`,
    ``,
    `• Action: ${action}`,
    `• Value: ${value || '(no ETH)'}`,
    `• Target: ${target}`,
    block_number ? `• Block: ${block_number} (pre-confirmation flashblock)` : `• Pre-confirmation flashblock`,
    ``,
    `Caught at flashblock speed — before canonical confirmation. This is FlashWatch running live on Base mainnet.`,
    ``,
    `[FlashWatch on GitHub](https://github.com/ortegarod/flashwatch)`,
  ];

  return lines.join('\n');
}

function postToMoltbook(alert) {
  const now = Date.now();
  const lastTime = lastPosted[alert.rule_name] || 0;
  if (now - lastTime < COOLDOWN_MS) {
    console.log(`[cooldown] Skipping ${alert.rule_name} — posted ${Math.round((now - lastTime) / 1000)}s ago`);
    return;
  }

  const content = formatAlert(alert);
  const title = (() => {
    const val = alert.tx.value_eth > 0 ? ` — ${alert.tx.value_eth.toFixed(2)} ETH` : '';
    const label = alert.tx.to_label ? ` → ${alert.tx.to_label}` : '';
    return `${alert.rule_name.replace(/-/g, ' ')}${val}${label} on Base`;
  })();

  const body = JSON.stringify({
    submolt_name: 'lablab',
    title,
    content,
  });

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
        console.log(`[moltbook] Posted: ${title}`);
      } else {
        console.error(`[moltbook] Error ${res.statusCode}: ${data.slice(0, 200)}`);
      }
    });
  });

  req.on('error', (e) => console.error('[moltbook] Request failed:', e.message));
  req.write(body);
  req.end();
}

const server = http.createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/webhook') {
    let body = '';
    req.on('data', chunk => body += chunk);
    req.on('end', () => {
      try {
        const alert = JSON.parse(body);
        console.log(`[alert] ${alert.rule_name} — ${alert.tx?.value_eth?.toFixed(4)} ETH → ${alert.tx?.to_label || alert.tx?.to}`);
        postToMoltbook(alert);
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
    res.end(JSON.stringify({ status: 'ok', service: 'flashwatch-moltbook-relay', cooldowns: lastPosted }));
  } else {
    res.writeHead(404);
    res.end('not found');
  }
});

server.listen(PORT, BIND, () => {
  console.log(`FlashWatch → Moltbook relay listening on http://${BIND}:${PORT}`);
  console.log(`Webhook endpoint: http://${BIND}:${PORT}/webhook`);
  console.log(`Moltbook key loaded: ${MOLTBOOK_API_KEY ? 'yes' : 'NO - will fail'}`);
});
