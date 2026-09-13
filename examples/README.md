# Examples

This directory contains reference material for building on Buzz beyond the desktop app and AI agents.

## `countdown-bot/`

A small non-AI bot that connects directly to the Buzz relay over WebSocket, authenticates with NIP-42, subscribes to one channel, and replies to deterministic commands like `!countdown 5` and `!fib 8`.

It demonstrates two identity paths:

1. **Standalone bot identity** — the bot authenticates with its own key and must be explicitly admitted to closed/allowlisted relays.
2. **Owner-attested / agent OAuth path** — the bot authenticates with its own key while presenting the same `BUZZ_AUTH_TAG` NIP-OA credential that Buzz agents receive from the owner/agent OAuth flow, so a relay can admit it because its owner is already a relay member.

See [`countdown-bot/README.md`](countdown-bot/README.md) for usage.

## `last30days-agent/`

A provider-agnostic multi-worker research example that implements `/last30days` semantics on top of existing ACP slash pass-through ([#919](https://github.com/block/buzz/pull/919)). Default model slug is DeepSeek V4 Pro; adopters supply their own API key and may point base URL, model, worker count, and evidence command anywhere OpenAI-compatible. No core routing changes.

See [`last30days-agent/README.md`](last30days-agent/README.md) for config, security notes, offline tests, and manual smoke steps. Proposed in [#4158](https://github.com/block/buzz/issues/4158).

## `meadow-core/`

A persona-pack example for Buzz agents.
