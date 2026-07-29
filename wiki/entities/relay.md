# Relay

The Nostr relay server is the single source of truth for a Buzz community. All reads and writes — chat messages, reactions, workflow triggers, canvas updates, git pushes, huddle events — flow through it as signed Nostr events.

Built with **Axum** on **Tokio**, the relay handles WebSocket connections, HTTP requests, and coordinates all subsystems. It imports all service crates (`buzz-db`, `buzz-auth`, `buzz-pubsub`, `buzz-search`, `buzz-audit`, `buzz-workflow`) and orchestrates cross-subsystem coordination.

**Key traits:**
- Single source of truth — no P2P gossip or replication
- Every action is a signed Nostr event
- All subsystems are coordinated through the relay, never directly between crates
- URL is authoritative — the relay's URL defines the community

**Related:**
- [Community](community) — tenant boundary
- [EventPipeline](../concepts/event-pipeline) — event lifecycle
- [buzz-relay](../components/buzz-relay) — implementation crate
