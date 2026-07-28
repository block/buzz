# Event Pipeline

Every Nostr event in Buzz goes through a 12-step pipeline when received by the relay:

1. **Auth check** — verify the client is authenticated (NIP-42)
2. **Pubkey match** — ensure the event pubkey matches the authenticated client
3. **Reject reserved kinds** — prevent spoofing of internal event types (KIND_AUTH)
4. **Ephemeral route** — ephemeral events are broadcast but not persisted
5. **Schnorr verify** — cryptographic signature verification
6. **Membership check** — verify sender is a member of the target channel
7. **DB insert** — idempotent write to Postgres (event ID is the dedup key)
8. **Redis publish** — publish to Redis pub/sub for cross-process notification
9. **Fan-out** — send event to all subscribed WebSocket connections
10. **Search index** — update Postgres FTS search index
11. **Audit log** — append to the hash-chain audit trail
12. **Workflow trigger** — evaluate workflow conditions and execute matching actions

**Related:**
- [NostrEvent](../entities/nostr-event)
- [Relay](../entities/relay)
- [buzz-relay](../components/buzz-relay)
