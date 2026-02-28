# BaseWhales - Ask Endpoint

> Real-time Base L2 whale monitoring. Query large transfers, DEX swaps, bridge activity, and wallet patterns detected from flashblocks. 0.01 USDC per query via x402.

## Endpoint

```
POST https://basewhales.com/api/ask
Content-Type: application/json

{ "question": "What are the biggest whale moves today?" }
```

No payment header -> HTTP 402 with full x402 payment spec.

## Payment

0.01 USDC on Base via x402. Install:

```bash
npm install @x402/fetch @x402/evm viem
```

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
  body: JSON.stringify({ question: 'What are the biggest whale moves today?' }),
});
const { answer, payment_tx, payment_explorer } = await res.json();
```

## Response

```json
{
  "answer": "AI-interpreted analysis...",
  "payment_tx": "0x...",
  "payment_explorer": "https://basescan.org/tx/0x..."
}
```

## Source

https://github.com/ortegarod/flashwatch
