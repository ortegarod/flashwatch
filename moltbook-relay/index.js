/**
 * FlashWatch auth proxy
 *
 * FlashWatch sends plain unauthenticated webhooks. OpenClaw requires
 * a Bearer token. This proxy is the bridge — it adds auth and forwards.
 *
 * All formatting + AI logic lives in ~/.openclaw/hooks/transforms/flashwatch.js
 */

const http = require('http');
const fs   = require('fs');
const path = require('path');

// ── Logging ───────────────────────────────────────────────────────────────────

const LOG_ENABLED = process.env.RELAY_LOG !== '0';
const LOG_PATH    = process.env.RELAY_LOG_PATH || path.join(__dirname, '..', 'alerts.log');

function logAlert(payload, openclawStatus) {
  if (!LOG_ENABLED) return;
  const entry = {
    ts: new Date().toISOString(),
    rule_name: payload.rule_name,
    value_eth: payload.tx?.value_eth,
    from: payload.tx?.from,
    to: payload.tx?.to,
    to_label: payload.tx?.to_label,
    tx_hash: payload.tx?.hash,
    basescan: payload.tx?.hash ? `https://basescan.org/tx/${payload.tx.hash}` : null,
    category: payload.tx?.category,
    block: payload.block_number,
    openclaw_status: openclawStatus,
  };
  fs.appendFileSync(LOG_PATH, JSON.stringify(entry) + '\n');
}

const PORT = process.env.RELAY_PORT || 4747;
const BIND = process.env.RELAY_BIND || '127.0.0.1';

const CREDS = JSON.parse(fs.readFileSync(
  path.join(process.env.HOME, '.config/flashwatch/credentials.json'), 'utf8'
));

const TOKEN       = CREDS.hooks_token;
const OPENCLAW_URL = new URL(CREDS.openclaw_url || 'http://127.0.0.1:18789');
const TARGET_PATH  = '/hooks/flashwatch';

if (!TOKEN) { console.error('hooks_token missing'); process.exit(1); }

http.createServer((req, res) => {
  if (req.method === 'GET' && req.url === '/health') {
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify({ status: 'ok' }));
  }

  if (req.method === 'POST' && req.url === '/webhook') {
    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => {
      const body = Buffer.concat(chunks);
      console.log(`[${new Date().toISOString()}] forwarding ${body.length}b → OpenClaw`);

      const parsed = JSON.parse(body);
      const fwd = http.request({
        hostname: OPENCLAW_URL.hostname,
        port:     OPENCLAW_URL.port || 18789,
        path:     TARGET_PATH,
        method:   'POST',
        headers: {
          'Authorization':  `Bearer ${TOKEN}`,
          'Content-Type':   'application/json',
          'Content-Length': body.length,
        },
      }, r => {
        let out = '';
        r.on('data', c => out += c);
        r.on('end', () => {
          console.log(`[openclaw] ${r.statusCode} ${out.slice(0, 80)}`);
          logAlert(parsed, r.statusCode);
        });
      });

      fwd.on('error', e => console.error('[openclaw] forward error:', e.message));
      fwd.write(body);
      fwd.end();

      res.writeHead(202);
      res.end('ok');
    });
    return;
  }

  res.writeHead(404);
  res.end();
}).listen(PORT, BIND, () => {
  console.log(`FlashWatch auth proxy → http://${BIND}:${PORT}`);
  console.log(`  Forwards to: ${OPENCLAW_URL.href}${TARGET_PATH}`);
});
