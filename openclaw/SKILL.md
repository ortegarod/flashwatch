---
name: flashwatch
description: "Monitor Base L2 flash blocks in real time, set up custom alerts via webhook, and act on on-chain events autonomously. Use when: starting/stopping FlashWatch, configuring alert rules for whale transfers/DEX swaps/bridge activity/address watches, receiving on-chain alerts in OpenClaw, or posting AI-interpreted alerts to Moltbook. NOT for: general crypto price data (use bankr), historical chain data (use RPC directly)."
metadata: {"openclaw":{"emoji":"⚡","requires":{"bins":["cargo","curl"]}}}
---

# FlashWatch Skill

Real-time Base flashblock monitor. Watches pre-confirmation blocks (~200ms before finalization), fires webhooks on rule matches, and routes alerts directly into OpenClaw for autonomous action (e.g. post to Moltbook).

**Repo:** `~/repos/flashwatch`
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
cd ~/repos/flashwatch
source ~/.cargo/env
cargo build --release
# Binary: target/release/flashwatch
```

Only needed once, or after code changes.

---

## Running

```bash
# Start (production rules, posts to OpenClaw)
./start.sh

# Low-threshold test mode (fires frequently, good for verifying end-to-end)
./start.sh --test
```

`start.sh` loads the OpenClaw hooks token from `~/.config/flashwatch/credentials.json` and sets it as `OPENCLAW_HOOKS_TOKEN`. The Rust binary adds it as a Bearer header on every webhook request.

**Check if running:**
```bash
pgrep -a flashwatch
```

**Stop:**
```bash
pkill -f 'flashwatch alert'
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
User=ubuntu
WorkingDirectory=/home/ubuntu/repos/flashwatch
ExecStart=/home/ubuntu/repos/flashwatch/start.sh
Restart=always
RestartSec=5
Environment=HOME=/home/ubuntu

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

## Alert Rules (rules-moltbook.toml)

```toml
[global]
cooldown_secs = 120
max_per_minute = 5

[[rules]]
name = "whale-transfer"
webhook = "http://127.0.0.1:18789/hooks/flashwatch"
cooldown_secs = 300
[rules.trigger]
kind = "large_value"
min_eth = 100.0
```

### Rule trigger types

| kind | what it matches |
|---|---|
| `large_value` | any tx with ETH value ≥ min_eth |
| `protocol` + `categories = ["dex"]` | DEX swaps |
| `protocol` + `categories = ["bridge"]` | bridge activity |
| `address` + `address = "0x..."` | specific wallet activity |

---

## Webhook Payload (what OpenClaw receives)

```json
{
  "rule_name": "whale-transfer",
  "block_number": 42682748,
  "flashblock_index": 2,
  "tx": {
    "hash": "0xabc...",
    "from": "0x1234...",
    "to": "0x5678...",
    "to_label": "Bybit Hot Wallet 6",
    "value_eth": 505.01,
    "category": "unknown"
  }
}
```

The transform at `~/.openclaw/hooks/transforms/flashwatch.js` formats this into an agent message. The agent then decides what to do.

---

## Posting Alerts to Moltbook

When an alert fires, use the post template at `~/repos/flashwatch/post-template.md`:

```
[emoji based on size] [value] ETH [action] on Base

[one-line analysis — who are the wallets, what does this signal]

🔗 https://basescan.org/tx/[tx_hash]
```

**Examples:**
```
🐋 505 ETH transferred on Base
Coinbase cold → hot rotation. Classic treasury management.
🔗 https://basescan.org/tx/0xabc...

🚨 1,200 ETH bridged from Ethereum → Base
Unknown wallet moving serious size to Base. New whale entering?
🔗 https://basescan.org/tx/0xdef...
```

Post to submolt `lablab` using the Moltbook API key at `~/.config/moltbook/credentials.json`.
Moltbook rate limit: 1 post per 30 minutes.

---

## Credentials

| File | Contains |
|---|---|
| `~/.config/flashwatch/credentials.json` | `hooks_token` (OpenClaw), `openclaw_url` |
| `~/.config/moltbook/credentials.json` | `api_key` (Moltbook) |

⚠️ Do NOT read these files — keys flow through Anthropic servers.

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

**Webhook 401:** Check that `hooks_token` in `~/.config/flashwatch/credentials.json` is correct and `OPENCLAW_HOOKS_TOKEN` env var is set.

**Process died:** Check `/tmp/flashwatch.log` or `journalctl -u flashwatch`. The binary auto-reconnects on WebSocket drops — if it exits entirely, systemd will restart it.
