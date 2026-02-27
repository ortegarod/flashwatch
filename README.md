# flashwatch

Real-time monitor and analyzer for [Base Flashblocks](https://docs.base.org/docs/tools/flashblocks) — the pre-confirmation block feed that gives you sub-second visibility into what's landing on Base L2 before blocks are finalized.

## What it does

Flashblocks are partial blocks streamed by the Base sequencer via WebSocket, arriving every ~200ms — before blocks are finalized. `flashwatch` connects to that feed and gives you:

- **Stream** — live flashblock feed with transaction details, gas stats, and decoded transfers
- **Monitor** — terminal dashboard with real-time metrics (block rate, gas price, tx throughput, latency)
- **Alert** — rule-based alerting on whale transfers, DEX swaps, bridge activity, and more
- **Serve** — web dashboard with live visualization, alert history, and REST API ← **start here**

## Repo Layout

```
flashwatch/
├── src/                    # Rust source — the binary
│   ├── main.rs             # CLI entry point (stream / monitor / alert / serve)
│   ├── stream.rs           # WebSocket connection to Base flashblocks feed
│   ├── rules.rs            # Rule engine — matches alerts against config
│   ├── alert.rs            # Webhook firing logic
│   ├── serve.rs            # Web dashboard + API server
│   ├── store.rs            # SQLite alert history
│   ├── decode.rs           # Transaction decoding (transfers, DEX, bridges)
│   └── ...
├── openclaw/               # OpenClaw integration — the AI layer
│   ├── SKILL.md            # Agent skill — instructions for your OpenClaw agent
│   └── hook-transform.js   # Hook transform — converts alert payload → agent message
├── static/
│   └── index.html          # Web dashboard UI (served by `flashwatch serve`)
├── rules.example.toml      # Example alert rules — copy and customize
├── start.sh                # Setup + launch script (installs OpenClaw files, starts serve)
└── Cargo.toml              # Rust package manifest
```

**The `openclaw/` directory is the integration layer that makes FlashWatch useful.** Without it you have a monitor that fires webhooks. With it, an AI agent receives every alert, researches the wallets, interprets the movement, and acts autonomously — no human required. `start.sh` wires everything up automatically.

## Install

Requires [Rust](https://rustup.rs/) 1.85+ (edition 2024).

```bash
git clone https://github.com/ortegarod/flashwatch
cd flashwatch
cargo build --release
```

## Quickstart (with OpenClaw)

### 1. Enable hooks in OpenClaw

FlashWatch POSTs alerts to OpenClaw's standard [`/hooks/agent`](https://docs.openclaw.ai/automation/webhook) endpoint. You just need hooks enabled in your OpenClaw config (`~/.openclaw/openclaw.json`):

```json
{
  "hooks": {
    "enabled": true,
    "token": "your-secret-token"
  }
}
```

If you've used OpenClaw webhooks before, you're already set.

### 2. Export your token

```bash
export OPENCLAW_HOOKS_TOKEN=your-secret-token
```

Same token as above. Add it to your shell profile so it persists.

### 3. Start

```bash
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

`flashwatch` integrates natively with [OpenClaw](https://openclaw.ai) using the standard [`/hooks/agent`](https://docs.openclaw.ai/automation/webhook) endpoint — no custom config or mapping required. When a rule fires, the Rust binary builds the full agent prompt from the alert data and POSTs it directly to OpenClaw.

```
Base flashblocks feed (200ms)
  → flashwatch (Rust) — rule matching, builds agent message
  → OpenClaw /hooks/agent (Bearer auth)
  → Isolated agent session — research wallets, interpret movement, post to Moltbook
```

The only requirement on the OpenClaw side is `hooks.enabled: true` and a token — standard webhook setup. No transforms, no mappings, no changes to your config beyond what you'd do for any OpenClaw webhook integration.

See `openclaw/SKILL.md` for the full setup and configuration reference.

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
