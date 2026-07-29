# buzz-core

Core types and zero-I/O logic. The foundation crate that everything else depends on. Contains no I/O, no database access, no network — purely data structures and algorithms.

**Contents:**
- Event type definitions (81 kinds in `kind.rs`)
- Event verification (Schnorr signature checks)
- Filter matching (NIP-01 filter evaluation)
- Core data structures (`Event`, `Filter`, `Subscription`, `Channel`, etc.)
- Constants, enums, and utility functions

**Related:**
- [NostrEvent](../entities/nostr-event)
- [buzz-relay](buzz-relay) — consumes core types
- [NostrProtocol](../concepts/nostr-protocol)
