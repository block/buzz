# Examples

This directory contains reference material for building on Buzz beyond the desktop app and AI agents.

## `countdown-bot/`

A small non-AI bot that connects directly to the Buzz relay over WebSocket, authenticates with NIP-42, subscribes to one channel, and replies to deterministic commands like `!countdown 5` and `!fib 8`.

It demonstrates two identity paths:

1. **Standalone bot identity** — the bot authenticates with its own key and must be explicitly admitted to closed/allowlisted relays.
2. **Owner-attested / agent OAuth path** — the bot authenticates with its own key while presenting the same `BUZZ_AUTH_TAG` NIP-OA credential that Buzz agents receive from the owner/agent OAuth flow, so a relay can admit it because its owner is already a relay member.

See [`countdown-bot/README.md`](countdown-bot/README.md) for usage.

## `meadow-core/`

A persona-pack example for Buzz agents.

## `demo-crew/`

A four-agent persona pack for running Buzz in front of a live audience: Lead
plans and delegates, Maker drafts and labels its assumptions, Challenger
red-teams the draft, and Guard sorts what was shared into data-security tiers
and names anything that should not have been typed.

Where `meadow-core` shows a team reviewing code, this one shows a team being
watched — short replies, visible handoffs, and disagreement left on screen.

See [`demo-crew/README.md`](demo-crew/README.md) for the setup notes that matter
for audience-facing agents (no shell, owner-only).
