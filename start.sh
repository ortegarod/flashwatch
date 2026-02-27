#!/bin/bash
# FlashWatch — build and run the monitor
# Usage: ./start.sh [--test]

set -e
cd "$(dirname "$0")"

# ── Build if needed ────────────────────────────────────────────────────────────
if [ ! -f target/release/flashwatch ]; then
  echo "Building flashwatch..."
  source ~/.cargo/env
  cargo build --release
fi

# ── Install OpenClaw skill ─────────────────────────────────────────────────────
# Symlinks SKILL.md so your OpenClaw agent can find and use it.
SKILL_DIR="$HOME/.openclaw/workspace/skills/flashwatch"
OPENCLAW_DIR="$(pwd)/openclaw"

mkdir -p "$SKILL_DIR"
ln -sf "$OPENCLAW_DIR/SKILL.md" "$SKILL_DIR/SKILL.md"
echo "✓ Installed skill → $SKILL_DIR/SKILL.md"

# ── Pick rules file ────────────────────────────────────────────────────────────
# Default: rules.toml (copy rules.example.toml and customize)
# Override: FLASHWATCH_RULES=my-rules.toml ./start.sh
RULES="${FLASHWATCH_RULES:-rules.toml}"
if [ "$1" == "--test" ]; then
  # Test mode uses a low threshold (e.g. 1 ETH) so alerts fire frequently.
  # Useful for verifying the full pipeline end-to-end without waiting for a real whale.
  # Create rules-test.toml from rules.example.toml with min_eth set very low.
  RULES="rules-test.toml"
  echo "⚠️  Test mode — using low-threshold rules ($RULES)"
fi

if [ ! -f "$RULES" ]; then
  echo "ERROR: rules file '$RULES' not found."
  echo "Copy rules.example.toml and customize it:"
  echo "  cp rules.example.toml rules.toml"
  echo "  # edit rules.toml: set webhook URL and thresholds"
  exit 1
fi

# ── Check OpenClaw token ──────────────────────────────────────────────────────
# FlashWatch POSTs alerts directly to OpenClaw's /hooks/agent endpoint.
# This token must match the hooks.token in your OpenClaw config.
if [ -z "$OPENCLAW_HOOKS_TOKEN" ]; then
  echo "ERROR: OPENCLAW_HOOKS_TOKEN is not set."
  echo ""
  echo "This must match the hooks.token in your OpenClaw config (~/.openclaw/openclaw.json)."
  echo "Export it before running:"
  echo "  export OPENCLAW_HOOKS_TOKEN=your-token-here"
  echo "  ./start.sh"
  exit 1
fi

BIND="${FLASHWATCH_BIND:-127.0.0.1}"
PORT="${FLASHWATCH_PORT:-3003}"

echo "Starting FlashWatch — dashboard at http://${BIND}:${PORT}"
./target/release/flashwatch serve \
  --rules "$RULES" \
  --bind "$BIND" \
  --port "$PORT"
