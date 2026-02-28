# BaseWhales — Agent Skill

> Real-time Base L2 whale intelligence. Ask anything, pay 0.01 USDC.

## What This Is

BaseWhales is an AI agent (Kyro) that monitors Base L2 flashblocks 24/7 and builds a live database of every significant whale movement. You can query that intelligence for 0.01 USDC per question.

## Endpoint

```
POST https://basewhales.com/api/ask
Content-Type: application/json
X-PAYMENT: <x402 payment header>

{ "question": "Your question about Base whale activity" }
```

## Payment (x402 — exact scheme)

| Field | Value |
|---|---|
| Asset | USDC on Base (`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`) |
| Price | 0.01 USDC per query |
| Network | `base` |
| Scheme | `exact` (EIP-3009 `transferWithAuthorization`) |
| Facilitator | `https://facilitator.x402.rs` |

No X-PAYMENT header → HTTP 402 with full payment spec.

## Response (HTTP 200)

```json
{
  "answer": "AI-interpreted analysis of recent Base whale activity...",
  "payment_tx": "0x...",
  "payment_explorer": "https://basescan.org/tx/0x..."
}
```

## Example — Node.js

```javascript
import { wrapFetchWithPaymentFromConfig } from '@x402/fetch';
import { ExactEvmSchemeV1 } from '@x402/evm/v1';
import { privateKeyToAccount } from 'viem/accounts';

const account = privateKeyToAccount('0xYOUR_PRIVATE_KEY');
const fetchWithPayment = wrapFetchWithPaymentFromConfig(fetch, {
  schemes: [{ network: 'base', client: new ExactEvmSchemeV1(account), x402Version: 1 }],
});

const res = await fetchWithPayment('https://basewhales.com/api/ask', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ question: 'What are the biggest whale moves in the last 24 hours?' }),
});
const { answer, payment_tx } = await res.json();
// answer: full AI analysis with wallet patterns, ETH amounts, interpretations
// payment_tx: on-chain proof of payment (Basescan)
```

## Example — OpenClaw Agent (exec)

```bash
# Install deps once
cd /tmp && npm install @x402/fetch @x402/evm viem

# Query BaseWhales
node -e "
import('@x402/fetch').then(async ({wrapFetchWithPaymentFromConfig}) => {
  const {ExactEvmSchemeV1} = await import('@x402/evm/v1');
  const {privateKeyToAccount} = await import('viem/accounts');
  const fs = await import('fs');
  const key = fs.readFileSync(process.env.HOME+'/.openclaw/credentials/.wallet-key','utf8').trim();
  const account = privateKeyToAccount(key.startsWith('0x')?key:'0x'+key);
  const fetch2 = wrapFetchWithPaymentFromConfig(fetch, {
    schemes: [{network:'base', client: new ExactEvmSchemeV1(account), x402Version:1}]
  });
  const res = await fetch2('https://basewhales.com/api/ask', {
    method:'POST', headers:{'Content-Type':'application/json'},
    body: JSON.stringify({question: 'What are the biggest whale moves today?'})
  });
  const data = await res.json();
  console.log(data.answer);
});
"
```

## What Kyro Knows

Every whale alert FlashWatch has detected in the last 24 hours:
- Wallet addresses (with known labels: Coinbase, Binance, Uniswap, etc.)
- ETH amounts and transaction hashes
- Protocol decoded (DEX swap, bridge deposit, direct transfer)
- Timestamps and block numbers
- Pattern analysis (repeated wallets, rapid sequences, staging behavior)

Kyro answers from this live database — not from training data.

## Good Questions to Ask

- "What are the biggest whale moves in the last 24 hours?"
- "Are there any wallets accumulating ETH right now?"
- "What's the most suspicious activity you've seen today?"
- "Any patterns in the recent whale movements?"
- "Which wallets are most active today on Base?"

## Dashboard

Live alerts and stats: **https://basewhales.com**

## Source

https://github.com/ortegarod/flashwatch
