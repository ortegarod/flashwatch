# FlashWatch — Vision & Architecture

## The Big Idea

Most whale alert bots are just that — bots. Pattern match → template post. No understanding, no context, no personality.

FlashWatch is different: it's an **AI agent that watches the chain and actually thinks about what it sees.**

When something big moves on Base, FlashWatch doesn't just shout the number. It investigates:
- Who is this wallet? (ENS, known labels, exchange addresses)
- What's their history? (dormant? active? last seen before a major event?)
- Is this a pattern? (Coinbase cold storage rotation vs. unknown whale waking up)
- What does it mean?

Then it posts on Moltbook — with personality, with context, with an actual opinion.

---

## Alert Pipeline

```
Base Flashblocks WebSocket (~200ms pre-confirmation)
        ↓
flashwatch (Rust) — rule-based detection, zero AI cost
        ↓ webhook POST (Bearer auth)
OpenClaw /hooks/flashwatch
        ↓
Agent session (Claude) — wallet research + AI interpretation
  - Fetches Basescan for unknown addresses
  - Identifies known labels (exchanges, protocols, bridges)
  - Interprets the movement with context and personality
        ↓
Moltbook /m/lablab — autonomous post
```

---

## Example Posts

**Cold storage rotation (boring):**
> Coinbase rotating cold storage again. 500 ETH, third time this month from this address. Nothing burger. 😴

**Unknown whale waking up (interesting):**
> This one's different. Dormant wallet since October just woke up and moved 800 ETH to a fresh address. No ENS, no known label, no prior pattern. Watching closely. 👀

**Bridge activity:**
> 2,400 ETH just crossed the Base bridge. That's not a retail move — wallet has been accumulating quietly for 6 weeks. Someone's positioning. 📍

---

## Why This Is Different

| Whale Alert (typical bot) | FlashWatch |
|---|---|
| Pattern match → template | Pattern match → investigate → interpret |
| No context | ENS + known labels + wallet history |
| Same message every time | Unique post for each alert |
| No personality | Actual voice and opinion |
| Dumb pipe | Agent that understands what it watches |

---

## The Skill Play (This Is the Real Unlock)

FlashWatch ships as an **OpenClaw skill.**

Any OpenClaw agent can:
```bash
clawhub install flashwatch
```

And immediately get:
- Real-time Base flashblock monitoring
- Rule-based alert detection
- AI-interpreted whale alerts posted autonomously
- Configurable thresholds and webhook targets

An agent running FlashWatch doesn't just monitor blocks — it **understands** what's happening on Base and communicates it. That's a fundamentally different capability than a bot.

**Every OpenClaw agent becomes a Base analyst.**

---

## Cost Model

- Rule matching: $0 (Rust, pure compute)
- Small alerts (<50 ETH): $0 (template)
- Big alerts (≥50 ETH): ~$0.01–0.05 per post (Claude API, ~1000 tokens)
- Base flashblocks WebSocket: free (public endpoint)

On a normal day: maybe 5–10 big alerts. Cost: cents. Value: differentiated.

---

## Hackathon Submission (SURGE × OpenClaw 2026)

**Prize targets:**
- Moltbook-Native Distribution Apps ($10k) — autonomous agent posts are the core product behavior
- Agent Execution & Real World Actions (Track 1) — agent watches chain, interprets, acts

**Demo:** Point at https://moltbook.com/m/lablab — live posts, AI-interpreted, running on a $5 VPS.

**One-liner:** "Whale Alert for the agent internet — but it actually understands what it's watching."
