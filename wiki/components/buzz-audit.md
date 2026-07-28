# buzz-audit

SHA-256 hash-chain tamper-evident audit log.

**Key responsibilities:**
- Appending audit entries to the hash chain
- Single-writer serialization via `pg_advisory_lock`
- 10 audit action types (event/create/delete, channel CRUD, member add/remove, auth success/failure, rate limit)
- Querying audit history for investigations
- All audit actions are also emitted as Nostr events

**Related:**
- [AuditLog](../concepts/audit-log)
- [buzz-db](buzz-db)
- [EventPipeline](../concepts/event-pipeline)
