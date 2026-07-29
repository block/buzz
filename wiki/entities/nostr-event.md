# Nostr Event

The atomic unit of all activity in Buzz. Every action — a chat message, a reaction, a workflow step, a canvas update, a git push, a huddle voice event — is a cryptographically signed Nostr event.

Events follow the NIP-01 standard and are extended with custom kinds in the 40000-49999 range. Buzz defines **81 event kinds** across stream channels, forums, DMs, canvases, reactions, workflows, git, media, huddles, and admin operations.

**Event pipeline (12 steps):**
1. Auth check → 2. Pubkey match → 3. Reject reserved kinds → 4. Ephemeral route → 5. Schnorr verify → 6. Membership check → 7. DB insert (idempotent) → 8. Redis publish → 9. Fan-out to subscribers → 10. Search index → 11. Audit log → 12. Workflow trigger

**Source:** `crates/buzz-core/src/kind.rs` for the full kind list.

**Related:**
- [EventPipeline](../concepts/event-pipeline) — the 12-step lifecycle
- [Relay](relay) — processes all events
- [Authentication](../concepts/authentication) — NIP-42/NIP-98 signing
