# Examples

This directory contains reference material for building on Sprout beyond the desktop app and AI agents.

## `countdown-bot/`

A small non-AI bot that connects directly to the Sprout relay over WebSocket, authenticates with NIP-42, subscribes to one channel, and replies to deterministic commands like `!countdown 5` and `!fib 8`.

It demonstrates two identity paths:

1. **Standalone bot identity** — the bot authenticates with its own key and must be explicitly admitted to closed/allowlisted relays.
2. **Owner-attested / agent OAuth path** — the bot authenticates with its own key while presenting the same `SPROUT_AUTH_TAG` NIP-OA credential that Sprout agents receive from the owner/agent OAuth flow, so a relay can admit it because its owner is already a relay member.

See [`countdown-bot/README.md`](countdown-bot/README.md) for usage.

## `meadow-core/`

A persona-pack example for Sprout agents.
