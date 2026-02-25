# flashwatch

Real-time monitor and analyzer for [Base Flashblocks](https://docs.base.org/docs/tools/flashblocks) — the pre-confirmation block feed that gives you sub-second visibility into what's landing on Base L2 before blocks are finalized.

## What it does

Flashblocks are partial blocks streamed by the Base sequencer via WebSocket, typically arriving every ~200ms. `flashwatch` connects to the feed and gives you:

- **Stream** — live flashblock feed with transaction details, gas stats, and decoded transfers
- **Monitor** — terminal dashboard with real-time metrics (block rate, gas price, tx throughput, latency)
- **Logs** — filter for specific contract addresses or event topics at flashblock speed
- **Track** — follow a transaction from submission → flashblock inclusion → canonical finality
- **Alert** — rule-based alerting on whale transfers, DEX swaps, bridge activity, and more
- **Serve** — web dashboard with live visualization, alert history, and REST API

## Install

Requires [Rust](https://rustup.rs/) 1.85+ (edition 2024).

```bash
git clone https://github.com/ortegarod/flashwatch
cd flashwatch
cargo build --release
./target/release/flashwatch --help
```

## Usage

All commands connect to the flashblocks WebSocket feed by default. No API key needed for the public Base endpoint.

```bash
# Stream live flashblocks
flashwatch stream

# Full transaction details
flashwatch stream --full-txs

# Terminal metrics dashboard
flashwatch monitor

# Filter logs by contract address
flashwatch logs --address 0x4200000000000000000000000000000000000006

# Track a specific transaction
flashwatch track 0xabc123...

# Chain info + flashblock status
flashwatch info

# Alert on rule matches (see rules.example.toml)
flashwatch alert --rules rules.toml

# Launch web dashboard on :3000
flashwatch serve --rules rules.toml
```

## Configuration

Copy the example env file and rules config:

```bash
cp .env.example .env
cp rules.example.toml rules.toml
```

Edit `rules.toml` to define your alert triggers. Rules support:
- `eth_transfer` — ETH transfers above a threshold
- `large_value` — any transaction above a value threshold
- `protocol` — DEX swaps, bridge activity, and other known protocol interactions
- `address` — watch a specific address

Alerts can fire to stdout or a webhook URL.

## Output formats

```bash
flashwatch --format pretty stream   # colored terminal output (default)
flashwatch --format json stream     # JSON lines for piping
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `BASE_WS_URL` | `wss://mainnet.flashblocks.base.org/ws` | Flashblocks WebSocket endpoint |
| `BASE_RPC_URL` | `https://mainnet.base.org` | Base HTTP RPC endpoint |

## Stack

- [alloy](https://github.com/alloy-rs/alloy) — Ethereum types and RPC
- [tokio](https://tokio.rs/) — async runtime
- [axum](https://github.com/tokio-rs/axum) — web server for the dashboard
- [rusqlite](https://github.com/rusqlite/rusqlite) — alert history storage
- [clap](https://github.com/clap-rs/clap) — CLI

## Moltbook Integration (OpenClaw)

FlashWatch includes a Moltbook relay that autonomously posts whale alerts to [Moltbook /m/lablab](https://moltbook.com/m/lablab) — the AI agent social network. This is the OpenClaw integration for the SURGE × OpenClaw Hackathon.

```bash
# Start flashwatch + Moltbook relay together
./start.sh

# Test with low thresholds (fires frequently for demo)
./start.sh --test
```

The relay (`moltbook-relay/index.js`) runs as a local webhook server. Flashwatch fires JSON webhooks on rule matches; the relay formats and posts them to Moltbook. Live Base flashblock activity → autonomous agent posts.

Rule files:
- `rules-moltbook.toml` — production thresholds (whale ≥5 ETH, large value ≥10 ETH, DEX ≥1 ETH, bridge ≥0.5 ETH)
- `rules-test-moltbook.toml` — low thresholds for testing the pipeline

## Built for

SURGE × OpenClaw Hackathon 2026 — [Kyro](https://moltbook.com/u/Kyro) + [Rodrigo Ortega](https://github.com/ortegarod)

## License

MIT
