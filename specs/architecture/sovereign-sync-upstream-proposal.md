# Upstream proposal: sovereign sync agreements (draft for issue #2997)

Ready-to-paste text for the upstream conversation. Author posts under
their own account; edit tone/links as desired. Everything claimed as
"running" has live evidence on `slusset/buzz#feature/local-relay`.

---

## Proposal: a declaration vocabulary for relay-to-relay trust (kind 30700)

This continues the earlier thread about sync between independently-owned
relays. We've been running an implementation for a while now and the
vocabulary has stabilized enough to propose for the shared registry.

### What it is

A **sync declaration** is one addressable event (provisionally kind
`30700`) expressing one half of a relay-to-relay relationship:

- `d = "<role>/<stream-id>"` where role is one of `export`, `admit`,
  `read`, `key-grant`, `steward`
- `n` tag names the **node** the declaration governs (one owner runs many
  nodes; journals replicate whole, so every copy carries declarations for
  every node and each node evaluates only its own)
- `p` tags carry counterparty/verification pubkeys; `e` tag pins the
  counterparty's declaration head (drift between heads breaks the match
  visibly)
- content carries `status`, a stable `principal` label, and role-specific
  fields (stream selection for `export`, powers for `steward`)

An **agreement** is a matched pair of declarations, one signed by each
party. No countersignature envelope: each side signs its own half, both
halves replicate over the streams they govern, and an unmatched or
drifted pair is an *observable* state rather than a hidden config
difference.

### Why upstream might care

1. **Config-as-events.** Our adapters derive their peer trust, stream
   exports, and reader grants by evaluating owner-signed declaration
   heads in their own store, with env/file config demoted to bootstrap.
   Operator trust survives redeploys because it lives in the journal, and
   every trust change is a signed, attributable, revocable event. The
   relay-mesh work already gives runtimes attested identities — this is
   the same idiom one level up, between independently-owned relays.
2. **It degrades to plain events.** Everything here is NIP-01 addressable
   events + filters. A relay that doesn't speak the vocabulary still
   custodies and serves it correctly. No new transport is required;
   replication ports are an optimization.
3. **Bindings map onto NIP-29.** A relationship between two relays is a
   two-member shared context scoped by an `h` tag; a community relay is
   the N-member case of the same object. An unmodified Buzz client can
   open a relay-to-relay binding as a channel. Solo-first nodes and
   community-first nodes join each other with one vocabulary.
4. **Blob custody inherits the vocabulary.** Artifact access follows
   reference: a content-addressed blob is served only to principals
   holding a read grant on a stream whose events `x`-tag it. No separate
   artifact ACLs.

### Running evidence (fork)

- Laptop relay and a Cloudflare DO adapter both rehydrate peer trust /
  exports / readers from declaration heads; a production redeploy with
  blank env vars retained trust from the journal.
- Two independently-keyed laptops + the CF rendezvous run bidirectional
  selective sync governed entirely by declarations; the offer → admit →
  pin → drift → re-pin lifecycle is exercised end to end.
- R2-backed artifact custody with reference-gated access is live behind
  the same grants.
- Full draft spec: `specs/architecture/sovereign-sync-agreement-v0.1-draft.md`
  on the fork.

### Asks

1. **Registry**: assign (or bless the provisional) `30700` for sync
   declarations in `buzz-core/src/kind.rs`.
2. **Grammar**: comment on the `d = role/stream` + `n`-tag convention —
   especially whether the node-scoping tag collides with any planned use
   of `n`.
3. **NIP-29 mapping**: sanity-check the shared-context-as-binding framing
   with the group-model owners.
4. **Enforcement strictness**: the open question from before — should a
   relay refuse unmatched streams, or serve them and surface the
   mismatch? We currently treat strictness as adapter policy and lean on
   observability (a read-only steward agent reports drift); interested in
   whether upstream wants a normative position.

Happy to carve any of this into a NIP-style doc under `docs/nips/` if
there's appetite.
