---
name: flashwatch
description: "Monitor Base L2 flash blocks in real time, set up custom alerts via webhook, and act on on-chain events autonomously. Use when: starting/stopping FlashWatch, configuring alert rules for whale transfers/DEX swaps/bridge activity/address watches, receiving on-chain alerts in OpenClaw, or posting AI-interpreted alerts to Moltbook. NOT for: historical chain data (use RPC directly)."
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
        ↓ webhook POST with Bearer token on rule match
  OpenClaw /hooks/flashwatch — transform fires, agent session receives alert
        ↓
  Agent acts: posts to Moltbook, sends notification, etc.
```

---

## Build

```bash
cd /path/to/flashwatch    # wherever you cloned the repo
source ~/.cargo/env       # if installed via rustup
cargo build --release
# Binary: target/release/flashwatch
```

Only needed once, or after code changes.

---

## Running

`start.sh` does three things every time you run it:
1. Builds the binary if it doesn't exist yet
2. Symlinks `openclaw/hook-transform.js` and `openclaw/SKILL.md` into your OpenClaw config
3. Starts `flashwatch serve` with your rules file and dashboard

```bash
# Normal start — uses rules.toml, dashboard at http://localhost:3003
./start.sh

# Test mode — uses rules-test.toml (set min_eth very low, e.g. 1 ETH)
# Use this to verify the full pipeline without waiting for a real whale alert
./start.sh --test
```

**Requires** `OPENCLAW_HOOKS_TOKEN` — this is the shared secret that lets FlashWatch authenticate its webhook POSTs to OpenClaw. Both sides must use the same token.

Find or set it in your OpenClaw config (`~/.openclaw/openclaw.json`):
```json
{
  "hooks": {
    "enabled": true,
    "token": "your-secret-token"
  }
}
```
If hooks aren't configured yet, add that block and run `openclaw gateway restart`.

Then export the token before starting:
```bash
export OPENCLAW_HOOKS_TOKEN=your-secret-token
./start.sh
```
Or add it to your shell profile (`~/.bashrc`, `~/.zshrc`) so it persists across sessions.

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

The process must stay alive to keep monitoring. Use systemd:

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

### Structure

```toml
[global]
cooldown_secs = 120   # minimum seconds between any two alerts (prevents floods)
max_per_minute = 5    # hard cap on alerts per minute across all rules

[[rules]]
name = "whale-transfer"                          # label shown in logs and alert payload
webhook = "http://127.0.0.1:18789/hooks/flashwatch"  # where to POST when this rule fires
cooldown_secs = 300                              # this rule specifically won't fire again for 5 min

[rules.trigger]
kind = "large_value"   # what to look for (see trigger types below)
min_eth = 100.0        # only fire if transaction value is ≥ 100 ETH
```

Each `[[rules]]` block is one alert. You can define as many as you want. When a flash block contains a transaction matching the trigger, FlashWatch POSTs the alert to the `webhook` URL with a `Authorization: Bearer` header.

### Trigger types

| `kind` | What it watches | Extra fields |
|---|---|---|
| `large_value` | Any ETH transfer ≥ threshold | `min_eth` |
| `protocol` + `categories = ["dex"]` | DEX swaps on known routers | `min_eth` (optional) |
| `protocol` + `categories = ["bridge"]` | Bridge deposits/withdrawals | `min_eth` (optional) |
| `address` | Activity from/to a specific wallet | `address = "0x..."`, `min_eth` (optional) |

### Cooldowns

Cooldowns prevent your agent from being spammed when the same wallet is active repeatedly. `global.cooldown_secs` applies across all rules; `rules.cooldown_secs` overrides it for a specific rule. During a hackathon or testing, lower these to 10–30 seconds. In production, 5 minutes per rule is reasonable.

---

## How Alerts Reach You

When a rule fires, FlashWatch POSTs a JSON payload to OpenClaw's hook endpoint. OpenClaw runs the transform at `openclaw/hook-transform.js`, which converts the raw payload into a natural-language agent message. You receive that message as your task — research the wallets, interpret the movement, and take action.

**You do not receive the raw JSON directly.** The transform handles the formatting. But you should understand what the underlying data contains so you can work with it effectively.

### What the payload contains

```json
{
  "rule_name": "whale-transfer",   // which rule fired — tells you what kind of event this is
  "block_number": 42682748,        // Base block number — use for Basescan lookups
  "flashblock_index": 2,           // position within the flash block (pre-confirmation)
  "tx": {
    "hash": "0xabc...",            // transaction hash — link to basescan.org/tx/<hash>
    "from": "0x1234...",           // sending wallet address
    "to": "0x5678...",             // receiving wallet address
    "to_label": "Bybit Hot Wallet 6",  // known label if recognized, null if unknown
    "value_eth": 505.01,           // ETH value of the transaction
    "category": "unknown"          // detected category: "dex", "bridge", "transfer", or "unknown"
  }
}
```

### What you do with it

The transform turns this into your instructions. Your job every time:
1. **Identify** the wallets — use `to_label` if available, otherwise research via Basescan
2. **Interpret** the movement — what does this transaction signal?
3. **Act** — post to Moltbook, send a notification, or whatever your rules say to do

If `to_label` is null and the wallet is unknown, that's the most interesting case — research it.

---

## Customizing What Happens on Alert

When a rule fires, FlashWatch POSTs to OpenClaw, which runs `openclaw/hook-transform.js` as an **isolated agent session**. That file is the integration layer — it receives the alert payload, builds your instructions, and your session executes them.

**This is where you define what your agent does with every alert.** The default `hook-transform.js` is an example that posts whale alerts to Moltbook. You can change it to do anything: send a Telegram message, write to a database, call a trading API, trigger another workflow.

To customize:
1. Edit `openclaw/hook-transform.js` — change the instructions in the `message` block
2. `start.sh` will symlink your updated file into OpenClaw on next run (or just edit it in place at `~/.openclaw/hooks/transforms/flashwatch.js`)
3. Changes take effect immediately — no restart needed

The hook runs as an isolated session, so it has full access to your OpenClaw tools and skills but doesn't interrupt your main session.

---

## Other Commands

```bash
# Live stream all flashblocks
./target/release/flashwatch stream

# Terminal dashboard
./target/release/flashwatch monitor

# Track a tx to finality
./target/release/flashwatch track 0xabc123...
```

---

## Troubleshooting

**No alerts firing:** Switch to `--test` mode first. Production rules (≥100 ETH) may take minutes to hours to fire.

**Webhook 401:** The `OPENCLAW_HOOKS_TOKEN` env var must match `hooks.token` in your OpenClaw config. Check both match and that OpenClaw has `hooks.enabled: true`.

**Process died:** Check `/tmp/flashwatch.log` or `journalctl -u flashwatch`. The binary auto-reconnects on WebSocket drops — if it exits entirely, systemd will restart it.
