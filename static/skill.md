# BaseWhales - Ask Endpoint

> Ask questions about whale activity on Base L2. 0.01 USDC per query via x402.
>
> BaseWhales monitors every Base flashblock in real time -- large ETH transfers, DEX swaps, bridge deposits, wallet patterns. An AI agent (Kyro) watches 24/7 and maintains a live database of significant movements. You query that database and get back an interpreted answer with wallet addresses, amounts, transaction hashes, and pattern analysis.

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
