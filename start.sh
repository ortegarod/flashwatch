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
RULES="rules-moltbook.toml"
if [ "$1" == "--test" ]; then
  RULES="rules-test-moltbook.toml"
  echo "⚠️  Using low-threshold TEST rules"
fi

# ── Load OpenClaw token ────────────────────────────────────────────────────────
CREDS="$HOME/.config/flashwatch/credentials.json"
if [ ! -f "$CREDS" ]; then
  echo "ERROR: credentials not found at $CREDS"
  echo "Create it with: {\"hooks_token\": \"...\", \"openclaw_url\": \"http://127.0.0.1:18789\"}"
  exit 1
fi
TOKEN=$(python3 -c "import json; d=json.load(open('$CREDS')); print(d['hooks_token'])")

BIND="${FLASHWATCH_BIND:-127.0.0.1}"
PORT="${FLASHWATCH_PORT:-3003}"

echo "Starting FlashWatch — dashboard at http://${BIND}:${PORT}"
OPENCLAW_HOOKS_TOKEN="$TOKEN" ./target/release/flashwatch serve \
  --rules "$RULES" \
  --bind "$BIND" \
  --port "$PORT"
