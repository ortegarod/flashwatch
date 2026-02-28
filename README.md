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

## `/api/ask` — Pay-Per-Query AI Endpoint (x402)

Any agent or HTTP client can pay 0.01 USDC on Base and get AI-interpreted analysis of recent whale activity — synchronously, in a single HTTP call.

```bash
# No payment header → 402 with payment requirements
curl -X POST https://basewhales.com/api/ask \
  -H "Content-Type: application/json" \
  -d '{"question":"What are the biggest whale moves in the last 24 hours?"}'
# → HTTP 402 + x402 payment spec

# With @x402/fetch (handles payment automatically)
import { wrapFetchWithPaymentFromConfig } from '@x402/fetch';
import { ExactEvmSchemeV1 } from '@x402/evm/v1';
import { privateKeyToAccount } from 'viem/accounts';

const account = privateKeyToAccount('0x...');
const fetchWithPayment = wrapFetchWithPaymentFromConfig(fetch, {
  schemes: [{ network: 'base', client: new ExactEvmSchemeV1(account), x402Version: 1 }],
});

const res = await fetchWithPayment('https://basewhales.com/api/ask', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ question: 'What are the biggest whale moves in the last 24 hours?' }),
});
const { answer } = await res.json();
```

**Payment spec:**
- Asset: USDC on Base (`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`)
- Price: 0.01 USDC per query
- Facilitator: `https://facilitator.x402.rs`
- Scheme: `exact` (EIP-3009 `transferWithAuthorization`)

**How it works under the hood:**
```
POST /api/ask
  → [1] No X-PAYMENT header? Return 402 + payment spec
  → [2] X-PAYMENT present? Verify with facilitator.x402.rs
  → [3] Payment valid? Forward question to OpenClaw /v1/chat/completions
  → [4] Agent queries live SQLite DB (24h alert history), interprets, answers
  → [5] Answer returned synchronously in HTTP response
```

The agent answering is the same one monitoring Base flashblocks 24/7 — it has full context on every whale move in the last 24 hours.

**Config (env vars for self-hosted deployments):**

| Variable | Default | Description |
|---|---|---|
| `X402_PAY_TO` | required | Wallet address to receive payments |
| `X402_ASSET` | `0x833589...` (USDC mainnet) | ERC-20 token address |
| `X402_NETWORK` | `base` | Chain name (`base`, `base-sepolia`) |
| `X402_PRICE` | `10000` | Amount in token decimals (10000 = 0.01 USDC) |
| `X402_TOKEN_NAME` | `USD Coin` | EIP-712 domain name (`USDC` on Sepolia) |
| `X402_FACILITATOR_URL` | `https://facilitator.x402.rs` | x402 facilitator |
| `X402_RESOURCE_URL` | `https://basewhales.com/api/ask` | Canonical resource URL |
| `OPENCLAW_PORT` | `18789` | OpenClaw gateway port |

---

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
