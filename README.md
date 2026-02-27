# flashwatch

Real-time monitor and analyzer for [Base Flashblocks](https://docs.base.org/docs/tools/flashblocks) — the pre-confirmation block feed that gives you sub-second visibility into what's landing on Base L2 before blocks are finalized.

## What it does

Flashblocks are partial blocks streamed by the Base sequencer via WebSocket, arriving every ~200ms — before blocks are finalized. `flashwatch` connects to that feed and gives you:

- **Stream** — live flashblock feed with transaction details, gas stats, and decoded transfers
- **Monitor** — terminal dashboard with real-time metrics (block rate, gas price, tx throughput, latency)
- **Alert** — rule-based alerting on whale transfers, DEX swaps, bridge activity, and more
- **Serve** — web dashboard with live visualization, alert history, and REST API ← **start here**

## Install

Requires [Rust](https://rustup.rs/) 1.85+ (edition 2024).

```bash
git clone https://github.com/ortegarod/flashwatch
cd flashwatch
cargo build --release
```

## Quickstart (with OpenClaw)

### 1. Get your OpenClaw hook token

FlashWatch posts alerts to OpenClaw via a webhook. OpenClaw must have hooks enabled, and you need its token.

Find it in your OpenClaw config (`~/.openclaw/openclaw.json`):

```json
{
  "hooks": {
    "enabled": true,
    "token": "your-token-here"
  }
}
```

If hooks aren't set up yet, add that block and restart OpenClaw (`openclaw gateway restart`).

### 2. Export your token

```bash
export OPENCLAW_HOOKS_TOKEN=your-token-here
```

FlashWatch and OpenClaw both use this env var — one token, set once. Add it to your shell profile (`~/.bashrc`, `~/.zshrc`) or pass it inline when starting.

### 3. Start

```bash
# Installs OpenClaw hook + skill, launches dashboard
./start.sh
```

Dashboard runs at **http://localhost:3003**. Alerts fire to OpenClaw automatically when rules match.

```bash
# Low-threshold test mode (fires frequently, good for verifying end-to-end)
./start.sh --test
```

## serve vs alert

`flashwatch serve` and `flashwatch alert` both watch flashblocks and fire webhooks when rules match. The difference:

| | `serve` | `alert` |
|---|---|---|
| Web dashboard | ✅ http://localhost:3003 | ❌ |
| Alert history (SQLite) | ✅ | ❌ |
| Webhook firing | ✅ | ✅ |
| Use case | Normal use — you want visibility | Headless / minimal footprint |

**Use `serve`.** Use `alert` only if you're on a resource-constrained machine and don't need the UI.

## Alert Rules

Rules are defined in TOML. Copy the example and customize:

```bash
cp rules.example.toml rules.toml
```

```toml
[global]
cooldown_secs = 120
max_per_minute = 5

[[rules]]
name = "whale-transfer"
webhook = "http://127.0.0.1:18789/hooks/flashwatch"  # OpenClaw endpoint
cooldown_secs = 300
[rules.trigger]
kind = "large_value"
min_eth = 100.0
```

Trigger types: `large_value`, `protocol` (categories: `dex`, `bridge`), `address`

## OpenClaw Integration

`flashwatch` integrates natively with [OpenClaw](https://openclaw.ai) — when a rule fires, it POSTs the alert directly to OpenClaw's hook endpoint with a Bearer token. OpenClaw routes it to an agent session for AI interpretation and autonomous posting to [Moltbook /m/lablab](https://moltbook.com/m/lablab).

```
Base flashblocks feed (200ms)
  → flashwatch (Rust) — rule matching
  → OpenClaw /hooks/flashwatch (Bearer auth)
  → Agent session — research wallets, interpret movement, post to Moltbook
```

`start.sh` handles everything: installs the OpenClaw hook transform and skill from `openclaw/`, loads your credentials, and starts the monitor.

See `openclaw/SKILL.md` for full agent instructions.

## Other Commands

```bash
# Live stream
./target/release/flashwatch stream
./target/release/flashwatch stream --full-txs

# Terminal metrics dashboard
./target/release/flashwatch monitor

# Filter by contract address
./target/release/flashwatch logs --address 0x4200...

# Track a tx to finality
./target/release/flashwatch track 0xabc123...
```

## Stack

- [alloy](https://github.com/alloy-rs/alloy) — Ethereum types and RPC
- [tokio](https://tokio.rs/) — async runtime
- [axum](https://github.com/tokio-rs/axum) — web dashboard
- [rusqlite](https://github.com/rusqlite/rusqlite) — alert history
- [clap](https://github.com/clap-rs/clap) — CLI

## Built for

SURGE × OpenClaw Hackathon 2026 — [Kyro](https://moltbook.com/u/Kyro) + [Rodrigo Ortega](https://github.com/ortegarod)

## License

MIT
