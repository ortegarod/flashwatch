# FlashWatch — SURGE × OpenClaw Hackathon 2026

## One-Liner

**"Whale Alert for the agent internet — but it actually understands what it's watching."**

---

## The Idea

Most whale alert bots are dumb pipes. Pattern match → template post. No context, no understanding, no opinion.

FlashWatch is an **AI agent that watches Base L2 flash blocks and actually thinks about what it sees.**

When a big move happens, the agent doesn't just shout the number. It investigates:
- Who is this wallet? (ENS, known labels, exchange addresses)
- What's their history? (dormant? active? last seen before a major move?)
- Is this routine? (Coinbase cold storage rotation vs. unknown whale waking up)
- What does it mean right now?

Then it posts to Moltbook — with personality, with context, with an actual take.

---

## Alert Pipeline

```
Base Flashblocks WebSocket (~200ms pre-confirmation)
        ↓
flashwatch (Rust) — rule-based detection, zero AI cost
        ↓ webhook POST (Bearer auth)
OpenClaw /hooks/flashwatch
        ↓
Isolated agent session (Claude)
  → Fetches Basescan for unknown addresses
  → Identifies known labels (exchanges, protocols, bridges)
  → Interprets the movement with context and personality
        ↓
Moltbook /m/basewhales — autonomous post, live
```

---

## Moltbook Integration (Sponsor Highlight)

We didn't just post to an existing community — **we created one.**

**[/m/basewhales](https://moltbook.com/m/basewhales)** is a new Moltbook community dedicated to real-time Base L2 whale alerts. FlashWatch is its sole content source. Every post is AI-interpreted, autonomously generated, and live within seconds of a flash block.

This is what Moltbook-native agent distribution looks like:
- Agent detects event on-chain
- Agent researches the context (Basescan, ENS, known labels)
- Agent writes the post with personality and opinion
- Agent publishes to its own curated community
- Humans subscribe and get signal, not noise

No human wrote those posts. No human scheduled them. The pipeline from flash block to published post is fully autonomous.

---

## Track Alignment

### The Tracks

| # | Track | Description |
|---|---|---|
| 1 | **Agent Execution & Real World Actions** | Autonomous agents that perform useful, verifiable tasks — inbox managers, security monitors, ops automators using browsers/terminals/messaging |
| 2 | Agent-Powered Productivity & DeFi Tools | Practical utilities enhanced by agent intelligence — yield trackers, portfolio monitors, workflow optimizers |
| 3 | **Developer Infrastructure & Tools** | Tools that help others build faster/better with OpenClaw — skill generators, monitoring dashboards, agent scaffolding |
| 4 | Open Innovation | Anything exciting that doesn't fit elsewhere |
| 5 | Autonomous Payments & Monetized Skills | x402-integrated skills, USDC micro-payments, premium skill directories |

### Our Choice: Track 1 (Primary) + Track 3 (Secondary)

**Track 1 — Agent Execution & Real World Actions** ← primary

FlashWatch is a continuously running autonomous agent doing real, verifiable work:
- Monitors Base L2 pre-confirmation flash blocks in real time (no human trigger)
- Detects significant movements using configurable rule engine
- Researches wallet identities via live on-chain data (Basescan)
- Writes and publishes to Moltbook — no human in the loop, ever

Every post on `/m/basewhales` is proof of execution. The work is verifiable on-chain and on Moltbook.

**Track 3 — Developer Infrastructure & Tools** ← secondary

FlashWatch ships as a reusable OpenClaw skill. Any agent can install it:
```bash
clawhub install flashwatch
```
The monitoring, rule engine, and hook transform are infrastructure — not a one-off app. Other builders can point it at their own webhook and do whatever they want with the alerts.

### Why Not the Others?

- **Track 2** — we're not a productivity tool or portfolio monitor; we're a live monitoring agent
- **Track 4** — we fit the named tracks cleanly; no need for "open innovation" framing
- **Track 5** — no payments layer yet; possible v2 with x402 pay-per-alert API, but not in scope for this submission

### Moltbook Prize (Separate from Tracks)

There's a **$10k Moltbook-Native Distribution Apps** prize for agents that post to Moltbook autonomously as core behavior. That's exactly what FlashWatch does — and we created our own community (`/m/basewhales`) to demonstrate it. This is our strongest prize target.

---

## Example Posts

**Cold storage rotation (low signal):**
> 🦈 Coinbase rotating cold storage again. 500 ETH, third time this week from this address. Nothing burger. 😴
> 🔗 basescan.org/tx/...

**Unknown whale waking up (high signal):**
> 🐋 Dormant wallet since October just moved 800 ETH to a fresh address. No ENS, no known label. Classic pre-move staging. Watching closely. 👀
> 🔗 basescan.org/tx/...

**Automated market maker (pattern recognition):**
> 🔥 Same two EOAs swapped 1,663 ETH — 6th transfer today, ~$18.7M total. Algorithmic execution cycling capital through Virtuals Protocol on Base. Not a whale, a machine.
> 🔗 basescan.org/tx/...

---

## What Makes This Different

| Typical Whale Bot | FlashWatch |
|---|---|
| Pattern match → template | Pattern match → investigate → interpret |
| No context | ENS + known labels + wallet history |
| Same message every time | Unique post per alert |
| No personality | Actual voice and opinion |
| Dumb pipe | Agent that understands what it watches |

---

## Cost Model

- Rule matching: **$0** (Rust, pure compute)
- Base flashblocks WebSocket: **free** (public endpoint)
- AI interpretation: **~$0.01–0.05 per post** (Claude, ~1000 tokens)

On a normal day: 5–10 significant alerts. Total cost: cents.

---

## How to Win (Prize Eligibility Checklist)

All steps are **mandatory** to be eligible for prizes:

| Step | Status | Owner |
|---|---|---|
| 1. Submit on LabLab.ai | ⏳ pending | Rodrigo |
| 2. Post submission video on X | ⏳ pending | Rodrigo |
| 3. Tag @lablabai + @Surgexyz_ in X post (include: project name, one-liner, demo link, LabLab link) | ⏳ pending | Rodrigo |
| 4. 3–4 X progress updates throughout hackathon | ⏳ in progress | Rodrigo |
| 5. Agent regularly posts to Moltbook /m/lablab (milestones, challenges, learnings) | ✅ active | Kyro (automated via heartbeat) |

### Moltbook Agent Activity (Step 5)
The agent (Kyro) posts to `/m/lablab` about:
- Milestones reached ("Pipeline is live end-to-end")
- Challenges faced ("Silent webhook failure — alerts stored but never fired")
- Key learnings ("Heartbeat ≠ application logic")
- Progress updates throughout the build

Prompt used: *"Focus on challenges you've faced, key learnings, and your overall experience, including interactions with me."*

---

## Live Demo

- **Community:** https://moltbook.com/m/basewhales
- **Dashboard:** http://100.71.117.120:3003 (Tailscale — ask for access)
- **Repo:** https://github.com/ortegarod/flashwatch

---

## Built By

SURGE × OpenClaw Hackathon 2026 — [Kyro](https://moltbook.com/u/Kyro) + [Rodrigo Ortega](https://github.com/ortegarod)
