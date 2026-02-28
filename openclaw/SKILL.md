---
name: flashwatch
description: "Monitor Base L2 flash blocks in real time and trigger autonomous agent actions on on-chain events. Use when: installing or running FlashWatch, configuring alert rules for whale transfers/DEX swaps/bridge activity/address watches, verifying the alert pipeline is working, or troubleshooting. NOT for: historical chain data (use RPC directly)."
metadata: {"openclaw":{"emoji":"🐋","requires":{"bins":["cargo","curl"]}}}
---

# FlashWatch Skill

Real-time Base flashblock monitor. Watches pre-confirmation blocks (~200ms before finalization), fires webhooks on rule matches, and routes alerts directly into OpenClaw for autonomous action (e.g. post to Moltbook).

**Remote:** https://github.com/ortegarod/flashwatch

---

## How It Works

```
Base Flashblocks WebSocket (~200ms pre-confirmation)
        ↓
  flashwatch (Rust binary) — rule-based detection, zero AI cost
        ↓ POST /hooks/agent with full agent message + Bearer token
  OpenClaw — fires an isolated agent turn
        ↓
  Isolated agent session — researches wallets, posts to Moltbook
```

FlashWatch uses OpenClaw's standard [`/hooks/agent`](https://docs.openclaw.ai/automation/webhook#post-hooksagent) endpoint. The Rust binary builds the full agent prompt from the alert data and POSTs it directly — no custom config, no transforms, no changes to your OpenClaw setup beyond having hooks enabled.

---

## Prerequisites

- [OpenClaw](https://openclaw.ai) installed and running with hooks enabled:
  ```json
  {
    "hooks": {
      "enabled": true,
      "token": "your-secret-token"
    }
  }
  ```
- `OPENCLAW_HOOKS_TOKEN` env var set to match that token
- [Rust](https://rustup.rs/) 1.85+

---

## Build

```bash
cd /path/to/flashwatch
source ~/.cargo/env       # if installed via rustup
cargo build --release
# Binary: target/release/flashwatch
```

Only needed once, or after code changes.

---

## Running

`start.sh` does three things every time you run it:
1. Builds the binary if it doesn't exist yet
2. Symlinks `openclaw/SKILL.md` into your OpenClaw workspace
3. Starts `flashwatch serve` with your rules file and dashboard

```bash
# Normal start — uses rules.toml, dashboard at http://localhost:3003
./start.sh

# Test mode — uses rules-test.toml (set min_eth very low, e.g. 1 ETH)
# Use this to verify the full pipeline without waiting for a real whale alert
./start.sh --test
```

**Requires** `OPENCLAW_HOOKS_TOKEN` — this is the shared secret that authenticates FlashWatch's webhook POSTs to OpenClaw. It must match `hooks.token` in your OpenClaw config.

```bash
export OPENCLAW_HOOKS_TOKEN=your-secret-token
./start.sh
```

Add it to your shell profile (`~/.bashrc`, `~/.zshrc`) so it persists across sessions.

**Override defaults:**
```bash
FLASHWATCH_RULES=my-rules.toml ./start.sh   # use a different rules file
FLASHWATCH_BIND=0.0.0.0 ./start.sh          # bind to all interfaces
FLASHWATCH_PORT=8080 ./start.sh             # use a different port
```

**Check if running:**
```bash
pgrep -a flashwatch
```

**Stop:**
```bash
# If running via start.sh directly:
pkill -f 'flashwatch serve'

# If running via systemd:
sudo systemctl stop flashwatch
```

---

## Keeping It Running (systemd)

```bash
sudo tee /etc/systemd/system/flashwatch.service > /dev/null <<EOF
[Unit]
Description=FlashWatch Base Flashblock Monitor
After=network.target

[Service]
User=YOUR_USER
WorkingDirectory=/path/to/flashwatch
ExecStart=/path/to/flashwatch/start.sh
Restart=always
RestartSec=5
Environment=HOME=/home/YOUR_USER
Environment=OPENCLAW_HOOKS_TOKEN=your-token-here

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable flashwatch
sudo systemctl start flashwatch
sudo systemctl status flashwatch
```

**Check logs:**
```bash
journalctl -u flashwatch -f
```

---

## Alert Rules

Rules live in a TOML file (copy `rules.example.toml` to get started). Each rule defines **what to watch for** and **where to send the alert** when it fires.

```toml
[global]
cooldown_secs = 120   # minimum seconds between any two alerts
max_per_minute = 5    # hard cap on alerts per minute across all rules

[[rules]]
name = "whale-transfer"
webhook = "http://127.0.0.1:18789/hooks/agent"   # OpenClaw /hooks/agent endpoint
cooldown_secs = 300

[rules.trigger]
kind = "large_value"
min_eth = 100.0
```

### Trigger types

| `kind` | What it watches | Extra fields |
|---|---|---|
| `large_value` | Any ETH transfer ≥ threshold | `min_eth` |
| `protocol` + `categories = ["dex"]` | DEX swaps on known routers | `min_eth` (optional) |
| `protocol` + `categories = ["bridge"]` | Bridge deposits/withdrawals | `min_eth` (optional) |
| `address` | Activity from/to a specific wallet | `address = "0x..."`, `min_eth` (optional) |

### Cooldowns

`global.cooldown_secs` applies across all rules; `rules.cooldown_secs` overrides it per rule.

---

## How the Alert Pipeline Works

When a rule fires, FlashWatch builds a full agent message in Rust (`src/alert.rs → build_agent_message`) and POSTs it to OpenClaw's `/hooks/agent` endpoint:

```json
{
  "message": "...(full agent prompt with wallet addresses, tx link, instructions)...",
  "name": "FlashWatch",
  "wakeMode": "now",
  "deliver": false
}
```

OpenClaw fires an **isolated agent turn** — it receives that message as its prompt, runs with full access to tools (web_fetch, exec, etc.), and acts autonomously. Your main session is never involved.

---

## Customizing What Happens on Alert

The agent prompt is built by `build_agent_message()` in `src/alert.rs`. By default it tells the agent to:
1. Research unknown wallets via Basescan
2. Interpret the on-chain movement
3. Post an AI-interpreted alert to Moltbook

To change the behavior, edit `build_agent_message()` in `src/alert.rs` and rebuild:
```bash
cargo build --release
```

Set `FLASHWATCH_MOLTBOOK_SUBMOLT` to target a different Moltbook community (default: `basewhales`).

---

## `/api/ask` — Pay-Per-Query Intelligence API (x402)

FlashWatch exposes an x402-gated endpoint that any agent can call to get AI analysis of recent Base whale activity. Pay 0.01 USDC, get a synchronous answer.

**From another OpenClaw agent:**

```javascript
// Install: npm install @x402/fetch @x402/evm viem
import { wrapFetchWithPaymentFromConfig } from '@x402/fetch';
import { ExactEvmSchemeV1 } from '@x402/evm/v1';
import { privateKeyToAccount } from 'viem/accounts';

const account = privateKeyToAccount('0xYOUR_PRIVATE_KEY');
const fetchWithPayment = wrapFetchWithPaymentFromConfig(fetch, {
  schemes: [{ network: 'base', client: new ExactEvmSchemeV1(account), x402Version: 1 }],
});

const res = await fetchWithPayment('https://basewhales.com/api/ask', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ question: 'What are the biggest whale moves in the last 24 hours?' }),
});
const { answer } = await res.json(); // AI answer with real on-chain data
```

**From curl (to see the 402 payment spec):**
```bash
curl -X POST https://basewhales.com/api/ask \
  -H "Content-Type: application/json" \
  -d '{"question":"test"}'
# → HTTP 402 with x402 payment requirements
```

**What the agent knows:** Every whale alert FlashWatch has detected in the last 24 hours — wallet addresses, ETH amounts, transaction hashes, timestamps, decoded protocol labels. It answers from live data, not hallucination.

**To enable on your own FlashWatch deployment**, set these env vars (see `README.md` for full reference):
```bash
export X402_PAY_TO=0xYOUR_WALLET
export OPENCLAW_PORT=18789   # default
```

And enable `/v1/chat/completions` in your OpenClaw config:
```json
{ "gateway": { "http": { "endpoints": { "chatCompletions": { "enabled": true } } } } }
```

---

## Other Commands

```bash
./target/release/flashwatch stream      # live stream all flashblocks
./target/release/flashwatch monitor     # terminal dashboard
./target/release/flashwatch track 0x…  # track a tx to finality
```

---

## Troubleshooting

**No alerts firing:** Use `--test` mode first. Production rules (≥100 ETH) may take minutes to hours to fire.

**Webhook 401:** `OPENCLAW_HOOKS_TOKEN` must match `hooks.token` in your OpenClaw config. Check both.

**Webhook 404:** OpenClaw must have `hooks.enabled: true` in its config.

**Process died:** Check `journalctl -u flashwatch`. The binary auto-reconnects on WebSocket drops.
