# Architecture

Buzz is a self-hostable team workspace built on a single Nostr relay as the source of truth.

## Layers

```
Clients (Desktop, Mobile, CLI, Agent)  ← WebSocket / HTTP
        │
   buzz-relay (Axum)
        │
   ┌────┼────┐
   │    │    │
Postgres Redis MinIO
```

**The relay** is the central coordinator. All subsystems (`buzz-db`, `buzz-auth`, `buzz-pubsub`, `buzz-search`, `buzz-audit`, `buzz-workflow`) are orchestrated by `buzz-relay` and never talk to each other directly. Cross-subsystem coordination happens only through the relay.

**Clients** connect via WebSocket (real-time) or HTTP (REST). Every action is a signed Nostr event.

**Data stores** — Postgres (events, channels, search, audit, workflows), Redis (pub/sub, presence, typing indicators), MinIO/S3 (media attachments).

## Design principles

- **Single source of truth** — the relay event log. No P2P gossip.
- **URL is the community** — one URL = one isolated workspace.
- **Membership is the only gate** — no roles, no permissions, no ACLs beyond channel membership.
- **Events are immutable** — once written, events are never modified.
- **Everything is an event** — messages, reactions, workflow steps, git pushes, huddles. 81 event kinds.

## Security model

- NIP-42 for WebSocket auth (challenge-response)
- NIP-98 for HTTP auth (Nostr-signed HTTP requests)
- Channel membership is the only access control
- Hash-chain audit log for tamper-evident record
- SSRF protection via `is_private_ip()` checks

**Related:**
- [Relay](../entities/relay)
- [EventPipeline](event-pipeline)
- [MultiTenancy](multi-tenancy)
- [NostrProtocol](nostr-protocol)
