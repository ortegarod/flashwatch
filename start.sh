#!/bin/bash
# FlashWatch + Moltbook Relay — start both services
# Usage: ./start.sh [--test]

set -e
cd "$(dirname "$0")"

if [ ! -f target/release/flashwatch ]; then
  echo "Building flashwatch..."
  source ~/.cargo/env
  cargo build --release
fi

RULES="rules-moltbook.toml"
if [ "$1" == "--test" ]; then
  RULES="rules-test-moltbook.toml"
  echo "⚠️  Using low-threshold TEST rules"
fi

RELAY_BIND="${RELAY_BIND:-127.0.0.1}"
RELAY_PORT="${RELAY_PORT:-4747}"

echo "Starting Moltbook relay..."
RELAY_BIND="$RELAY_BIND" RELAY_PORT="$RELAY_PORT" node moltbook-relay/index.js &
RELAY_PID=$!
sleep 1

# Verify relay is up
if ! curl -sf "http://${RELAY_BIND}:${RELAY_PORT}/health" > /dev/null; then
  echo "Relay failed to start"
  kill $RELAY_PID 2>/dev/null
  exit 1
fi
echo "✓ Relay up at http://${RELAY_BIND}:${RELAY_PORT}"

echo "Starting FlashWatch with rules: $RULES"
./target/release/flashwatch alert -R "$RULES"

# Cleanup relay on exit
kill $RELAY_PID 2>/dev/null
