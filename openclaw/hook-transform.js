/**
 * FlashWatch — OpenClaw hook transform
 *
 * FlashWatch POSTs raw JSON (addresses, ETH values, tx hashes) to OpenClaw's
 * mapped hook endpoint at /hooks/flashwatch. OpenClaw runs this transform on
 * that payload and uses the returned `message` to fire an isolated agent turn
 * (equivalent to POST /hooks/agent). The isolated agent turn receives that
 * message and executes it — your main session is never involved.
 *
 * This file defines what the isolated agent turn is told to do.
 * Customize it to fit your use case — the example below researches wallets
 * via Basescan and posts AI-interpreted alerts to Moltbook.
 * Set FLASHWATCH_MOLTBOOK_SUBMOLT env var to target your own community.
 */

// Target Moltbook community. Override with FLASHWATCH_MOLTBOOK_SUBMOLT env var.
const MOLTBOOK_SUBMOLT = process.env.FLASHWATCH_MOLTBOOK_SUBMOLT || 'general';

// Well-known Base/Ethereum addresses. Add your own as you discover them.
const KNOWN_ADDRESSES = {
  '0x71660c4005ba85c37ccec55d0c4493e66fe775d3': 'Coinbase Hot Wallet',
  '0xa9d1e08c7793af67e9d92fe308d5697fb81d3e43': 'Coinbase Cold Storage',
  '0x503828976d22510aad0201ac7ec88293211d23da': 'Coinbase 2',
  '0xddfabcdc4d8ffc6d5beaf154f18b778f892a0740': 'Coinbase 3',
  '0x28c6c06298d514db089934071355e5743bf21d60': 'Binance Hot Wallet',
  '0x21a31ee1afc51d94c2efccaa2092ad1028285549': 'Binance Cold Wallet',
  '0x3154cf16ccdb4c6d922629664174b904d80f2c35': 'Base Bridge (L1)',
  '0x4200000000000000000000000000000000000010': 'Base L2 Bridge',
  '0x2626664c2603336e57b271c5c0b26f421741e481': 'Uniswap V3 Router (Base)',
  '0x198ef1ec325a96cc354c7266a038be8b5c558f67': 'Uniswap Universal Router (Base)',
  '0x833589fcd6edb6e08f4c7c32d4f71b54bda02913': 'USDC (Base)',
};

function label(addr) {
  if (!addr) return null;
  return KNOWN_ADDRESSES[addr.toLowerCase()] || null;
}

function fmt(addr) {
  if (!addr) return 'unknown';
  const l = label(addr);
  return l ? `${addr} (${l})` : addr;
}

module.exports = function flashwatchTransform(ctx) {
  const { payload } = ctx;
  if (!payload || !payload.rule_name || !payload.tx) return null;

  const rule = payload.rule_name;
  const tx   = payload.tx;

  const fromLabel = tx.from ? (label(tx.from) || null) : null;
  const toLabel   = tx.to_label || (tx.to ? label(tx.to) : null) || null;
  const from      = fmt(tx.from);
  const to        = toLabel ? `${tx.to} (${toLabel})` : fmt(tx.to);
  const value     = tx.value_eth != null ? `${tx.value_eth.toFixed(2)} ETH` : 'unknown amount';
  const block     = payload.block_number ? `block ${payload.block_number} fb${payload.flashblock_index}` : '';
  const txLink    = tx.hash ? `https://basescan.org/tx/${tx.hash}` : null;
  const basescanFrom = tx.from ? `https://basescan.org/address/${tx.from}` : null;
  const basescanTo   = tx.to  ? `https://basescan.org/address/${tx.to}`   : null;

  const message = [
    `[FlashWatch Alert — Base Mainnet]`,
    `${value} | Rule: ${rule} | ${block}`,
    `From: ${from}`,
    `To:   ${to}`,
    txLink ? `Tx: ${txLink}` : null,
    basescanFrom && !fromLabel ? `From profile: ${basescanFrom}` : null,
    basescanTo   && !toLabel   ? `To profile:   ${basescanTo}` : null,
    ``,
    `== YOUR JOB ==`,
    ``,
    `1. RESEARCH the wallets if they're unknown.`,
    `   - Fetch the Basescan address pages above using web_fetch`,
    `   - Look for tags, contract names, ENS names, transaction patterns`,
    `   - Is this a known exchange, protocol, whale, or DAO?`,
    `   - Is it a contract or an EOA? What has this address done before?`,
    ``,
    `2. INTERPRET the movement. Ask yourself:`,
    `   - Exchange deposit/withdrawal? → signals buy/sell intent`,
    `   - Cold storage rotation? → routine treasury management, low signal`,
    `   - Bridge deposit? → capital moving cross-chain, where and why?`,
    `   - Unknown wallet moving big size? → who is this and why now?`,
    `   - Anything unusual about the timing, size, or counterparty?`,
    ``,
    `3. WRITE a Moltbook post. Keep it under 280 characters + link. Format:`,
    `   [emoji] [value] ETH [what happened] on Base`,
    `   [one-line interpretation — confident, specific, have a take]`,
    `   🔗 [tx link]`,
    ``,
    `   Emoji guide: 🐋 huge (500+ ETH) | 🦈 large (200-500) | 🔥 interesting (100-200)`,
    `   Be direct. If you don't know who the wallet is, say so honestly.`,
    `   Don't make up labels. Don't pad with generic phrases.`,
    ``,
    `4. POST to Moltbook using the API (posting to /m/${MOLTBOOK_SUBMOLT}):`,
    `   Key at: ~/.config/moltbook/credentials.json (use exec, do NOT read the file directly)`,
    `   MOLTKEY=$(python3 -c "import json,os; d=json.load(open(os.path.expanduser('~/.config/moltbook/credentials.json'))); print(d['api_key'])")`,
    `   RESPONSE=$(curl -s -X POST "https://www.moltbook.com/api/v1/posts" \\`,
    `     -H "Authorization: Bearer $MOLTKEY" \\`,
    `     -H "Content-Type: application/json" \\`,
    `     -d '{"submolt":"${MOLTBOOK_SUBMOLT}","title":"[your title]","content":"[your post]"}')`,
    `   echo "$RESPONSE"`,
    ``,
    `   Rate limit: 1 post per 30 min. If you get a 429, log it and skip — don't retry.`,
    ``,
    `5. CHECK FOR VERIFICATION CHALLENGE:`,
    `   Some communities require a math verification to prevent spam. Check the response:`,
    `   CODE=$(echo "$RESPONSE" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('post',{}).get('verification',{}).get('verification_code',''))")`,
    `   CHALLENGE=$(echo "$RESPONSE" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('post',{}).get('verification',{}).get('challenge_text',''))")`,
    ``,
    `   If CODE is non-empty, you must verify:`,
    `   - Read the CHALLENGE text carefully — it's an obfuscated math word problem`,
    `   - Decode the words (e.g. "ThIrTy TwO" = 32, "SeVeN" = 7, "NoOtOnS" = newtons)`,
    `   - Solve the arithmetic and format to 2 decimal places (e.g. "25.00")`,
    `   - Submit: curl -s -X POST "https://www.moltbook.com/api/v1/verify" \\`,
    `       -H "Authorization: Bearer $MOLTKEY" \\`,
    `       -H "Content-Type: application/json" \\`,
    `       -d '{"verification_code":"'$CODE'","answer":"[your answer]"}'`,
  ].filter(Boolean).join('\n');

  return { message, name: 'FlashWatch' };
};
