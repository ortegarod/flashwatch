# BaseWhales — Agent Skill

> Real-time Base L2 whale intelligence. Ask anything, pay 0.01 USDC.

## What This Is

BaseWhales is an AI agent (Kyro) that monitors Base L2 flashblocks 24/7 and builds a live database of every significant whale movement. You can query that intelligence for 0.01 USDC per question via x402.

## How to Query

Install dependencies:

```bash
npm install @x402/fetch @x402/evm viem
```

Then make the request — x402 handles the payment automatically:

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
const { answer, payment_tx, payment_explorer } = await res.json();
console.log(answer);
// payment_tx is the on-chain Basescan link proving the payment settled
```

That's it. The fetch wrapper catches the 402, signs the payment, and retries — no manual header construction needed.

## Endpoint

```
POST https://basewhales.com/api/ask
Content-Type: application/json

{ "question": "Your question about Base whale activity" }
```

No payment header → HTTP 402 with full payment spec returned automatically.

## Payment Details (x402 — exact scheme)

| Field | Value |
|---|---|
| Asset | USDC on Base (`0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913`) |
| Price | 0.01 USDC per query |
| Network | `base` |
| Scheme | `exact` (EIP-3009 `transferWithAuthorization`) |
| Facilitator | `https://facilitator.x402.rs` |

## Response (HTTP 200)

```json
{
  "answer": "AI-interpreted analysis of recent Base whale activity...",
  "payment_tx": "0x...",
  "payment_explorer": "https://basescan.org/tx/0x..."
}
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
