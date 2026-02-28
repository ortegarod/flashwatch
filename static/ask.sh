#!/usr/bin/env bash
# BaseWhales — Ask the whale expert. Pays 0.01 USDC via x402.
# Usage: ./ask.sh "Your question" /path/to/wallet-key
#    or: curl -sL basewhales.com/ask.sh | bash -s -- "question" ~/key
set -euo pipefail

QUESTION="${1:-What are the biggest whale moves in the last 24 hours?}"
KEYFILE="${2:-$HOME/.openclaw/credentials/.wallet-key}"

if [ ! -f "$KEYFILE" ]; then
  echo "❌ Wallet key not found at $KEYFILE"
  echo "Usage: ./ask.sh \"question\" /path/to/private-key"
  exit 1
fi

KEYFILE="$(cd "$(dirname "$KEYFILE")" && pwd)/$(basename "$KEYFILE")"

# One-time dep install (cached in /tmp)
D=/tmp/basewhales-deps
if [ ! -d "$D/node_modules/@x402" ]; then
  echo "📦 Installing x402 deps (one-time)..." >&2
  mkdir -p "$D" && cd "$D"
  echo '{"name":"bw","type":"module","version":"1.0.0"}' > package.json
  npm install --silent @x402/fetch @x402/evm viem 2>&1 | tail -1 >&2
fi

# Write query script INTO deps dir (Node resolves modules relative to script)
cat > "$D/q.mjs" << ENDSCRIPT
import { wrapFetchWithPaymentFromConfig } from '@x402/fetch';
import { ExactEvmSchemeV1 } from '@x402/evm/v1';
import { privateKeyToAccount } from 'viem/accounts';
import fs from 'fs';
const key = fs.readFileSync('${KEYFILE}', 'utf8').trim();
const account = privateKeyToAccount(key.startsWith('0x') ? key : '0x' + key);
const f = wrapFetchWithPaymentFromConfig(fetch, {
  schemes: [{ network: 'base-sepolia', client: new ExactEvmSchemeV1(account), x402Version: 1 }],
});
const res = await f('https://basewhales.com/api/ask', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ question: \`${QUESTION}\` }),
});
const data = await res.json();
if (res.ok) {
  console.log(data.answer);
  if (data.payment_tx) console.log('\\n💳 Payment:', data.payment_explorer);
} else {
  console.error('Error:', JSON.stringify(data, null, 2));
  process.exit(1);
}
ENDSCRIPT

echo "🐋 Asking BaseWhales: \"$QUESTION\"" >&2
echo "" >&2
node "$D/q.mjs"
