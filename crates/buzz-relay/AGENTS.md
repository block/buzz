# Relay instructions

Read the root [`AGENTS.md`](../../AGENTS.md) first. For the public system model,
see [`ARCHITECTURE.md`](../../ARCHITECTURE.md) and
[`CONTRIBUTING.md`](../../CONTRIBUTING.md).

- Model product operations as signed Nostr events and use the shared ingest
  pipeline before adding a relay-specific HTTP endpoint. The supported HTTP
  bridge is documented in the architecture.
- Add or change event kinds in
  [`crates/buzz-core/src/kind.rs`](../buzz-core/src/kind.rs) before wiring relay
  behavior.
- Channel-scoped events and filters use the NIP-29 `h` group tag, not an `e`
  tag. Relay queries must include explicit `kinds` filters.
- Keep the thread materialization invariant: inserting or removing replies
  updates the affected `reply_count` and root `descendant_count`.

Run `just test-integration` (or the full `just test`) for relay, database, or
auth changes, in addition to targeted tests and the checks required by the root
instructions.
