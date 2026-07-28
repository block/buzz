# Audit Log

Buzz maintains a tamper-evident audit trail using a SHA-256 hash chain.

- Each audit entry contains: timestamp, action type, actor pubkey, target, previous hash, and metadata
- The chain is single-writer — serialized via `pg_advisory_lock` to prevent concurrent writes
- 10 audit action types: `EventCreated`, `EventDeleted`, `ChannelCreated`, `ChannelUpdated`, `ChannelDeleted`, `MemberAdded`, `MemberRemoved`, `AuthSuccess`, `AuthFailure`, `RateLimitExceeded`
- The hash chain ensures that modifying any historical entry would invalidate all subsequent hashes
- All audit actions are also Nostr events, making them visible in the relay's event stream

**Source:** `buzz-audit` crate

**Related:**
- [buzz-audit](../components/buzz-audit)
- [EventPipeline](event-pipeline)
- [Architecture](architecture)
