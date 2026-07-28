# Nostr Protocol

Buzz is built on the Nostr protocol — an open, decentralized protocol for signed events.

## Standard Compliance

- Wire-compatible with NIP-01 (event format, relay-client protocol)
- NIP-42 (authentication via challenge-response)
- NIP-50 (search)
- NIP-98 (HTTP auth)
- NIP-34 (git integration)

## Custom Extensions

Buzz extends Nostr with 81 event kinds in the 40000-49999 custom range, covering: stream messages, forum posts, DMs, canvases, reactions, workflows, git operations, media, huddles, typing indicators, and admin actions.

## Key Differences from P2P Nostr

Unlike typical Nostr relays, Buzz is **not P2P**. There is no gossip or replication between relays. The relay is the single source of truth. This is intentional — Buzz is a workspace, not a decentralized social network. The Nostr protocol is used for its cryptographic identity and event model, not for federation.

**Related:**
- [NostrEvent](../entities/nostr-event)
- [Authentication](authentication)
- [GitIntegration](git-integration)
- [EventPipeline](event-pipeline)
