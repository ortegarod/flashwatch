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

# ── Install OpenClaw hook + skill ─────────────────────────────────────────────
HOOK_DIR="$HOME/.openclaw/hooks/transforms"
SKILL_DIR="$HOME/.openclaw/workspace/skills/flashwatch"
OPENCLAW_DIR="$(pwd)/openclaw"

# Hook transform: tells the agent what to do when an alert fires
mkdir -p "$HOOK_DIR"
ln -sf "$OPENCLAW_DIR/hook-transform.js" "$HOOK_DIR/flashwatch.js"
echo "✓ Installed hook → $HOOK_DIR/flashwatch.js"

# Skill: agent reads this to understand how to run and operate FlashWatch
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
# OPENCLAW_HOOKS_TOKEN must match the token in your OpenClaw config (hooks.token).
# Set it in your environment or pass it inline: OPENCLAW_HOOKS_TOKEN=... ./start.sh
if [ -z "$OPENCLAW_HOOKS_TOKEN" ]; then
  echo "ERROR: OPENCLAW_HOOKS_TOKEN is not set."
  echo ""
  echo "Set it to the hooks token from your OpenClaw config (~/.openclaw/openclaw.json):"
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
