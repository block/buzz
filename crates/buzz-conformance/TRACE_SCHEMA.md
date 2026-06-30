# Trace Schema (`buzz-conformance`)

Schema version: **1** (`SCHEMA_VERSION` in `src/lib.rs`).

This document is the contract between the relay's emitter and the
independent replay checker. It is grounded in
[`docs/spec/MultiTenantRelay.tla`](../../docs/spec/MultiTenantRelay.tla)
and the runtime-formal-compliance skill. If you change the schema, this
file changes in the same commit.

## North star

> Don't ask "did the model pass." Ask "did the running code emit a trace
> the model accepts."

The relay emits one `TraceStep` per decision at the ingest/auth/read
seam. The checker replays the trace against a Rust re-implementation of
the spec's `Next` relation — it does **not** call any production
reducer.

## What a step looks like

```jsonc
{
  "schema": 1,
  "action": { /* TraceAction — see below */ },
  "state": {
    "resolved_community": "<uuid>",      // from TenantContext::community()
    "bound_host":         "<host str>",  // from TenantContext::host()
    "actor":              "<16 hex>"     // first 16 hex of authed pubkey
  }
}
```

`state` is *projected* state, not raw state. Concretely:

| Field | What it carries | What it does NOT carry |
|-------|-----------------|------------------------|
| `resolved_community` | server-resolved community UUID | client-claimed `h` tag, event id, payload |
| `bound_host` | opaque host string from the resolver | raw `Host` header bytes |
| `actor` | first 16 hex chars of the authed pubkey | private key, NIP-98 token, signature |

The `actor` prefix is a *hash already* from the client's POV (Schnorr
X-only) — so the prefix discloses nothing the relay's existing logs
don't already. This avoids dragging a hash dep into observability code.

## Actions

The `TraceAction` enum mirrors the spec's `Next` relation
(`MultiTenantRelay.tla:933+`). Each variant is documented with the
exact spec line it grounds in.

### Write seam

- **`write_insert { msg_id, channel, claimed_community }`**
  spec: `WriteInsert` (line 514). A successful per-channel insert. The
  row's community is `ChannelCommunity(channel)` per spec — the checker
  looks it up from the model, so there is no `row_community` field on
  the action. `claimed_community` is recorded so the checker can bite
  when the client's `h` tag disagrees with `ChannelCommunity(channel)`.

- **`write_insert_global { msg_id, claimed_community }`**
  spec: `WriteInsertGlobal` (line 562). Channel-less write (DM,
  gift-wrap, etc.). The row's community is derived from `bound_host`
  via the host-community map; no `channel` field. `claimed_community`
  recorded for the same reason as above.

- **`write_duplicate { msg_id, channel, claimed_community }`**
  spec: `WriteDuplicate` (line 612). The DB returned "already present";
  no row was added. No `row_community` because no row was produced.

### Read seam

- **`auth_check { channel, claimed_community, verdict }`**
  spec: `AuthCheck` (line 794). M2/M8 target this action. The checker
  enforces that `Allow` requires the channel's community ==
  `resolved_community` (the host-channel fence) AND the actor has scope
  for that channel.

- **`read_message_rows { channel, row_communities }`**
  spec: `ReadMessageRows` (line 643). Bulk read returning candidate
  rows. `row_communities` is a non-deduped `Vec` — the checker must see
  every leaked label, not the set.

- **`read_by_id_rows { channel, row_communities }`**
  spec: `ReadByIdRows` (line 681). The search lane emits this for each
  refetched hit. Modeling search as `read_message_rows` (candidates) +
  `read_by_id_rows` per hit makes the per-hit re-auth visible to the
  checker.

- **`read_host_feed_rows { row_communities }`**
  spec: `ReadHostFeedRows`. Kinds-only feed read derived from
  `bound_host`.

### Error seam

- **`sanitized_error { reason }`** where `reason ∈ { restricted,
  invalid, server_error }`. spec: `Inv_SanitizedErrors`, M6 mutation
  (line 778). The alphabet is **closed**: if `IngestError` ever grows a
  fourth variant, `sanitized_reason_for` (in
  `crates/buzz-relay/src/conformance/mod.rs`) goes non-exhaustive and
  CI catches it.

### Coverage breach

- **`impl_bug { kind }`** is not a spec action — it's a runtime witness
  that a critical seam exited without recording any other action. The
  checker treats it as a coverage breach and fails closed. Emitted by
  `EmitGuard::Drop` when the seam's counting tracer saw zero emits.

## Three projection rules that are load-bearing

These are the places a buggy relay could emit an in-spec trace if you
normalized away the violation. The checker assumes you *did not*.

1. **`claimed_community` is recorded separately from
   `resolved_community`.** If they ever disagree, the spec says
   "resolved wins"; the trace must show both so M2 (claimed-driven
   auth) can bite.

2. **`row_communities` is a `Vec`, not a `Set`, and is not filtered to
   the resolved tenant.** If two rows in the result set belong to
   different communities, the checker must see both labels — otherwise
   it cannot fail closed on `Inv_ReadConfinement`.

3. **`SanitizedReason` is a closed alphabet of three.** The relay's
   `IngestError` variants map 1:1 onto it. A fourth variant is a CI
   failure, not a silent bucket.

## Where the emitter lives

| File | What it emits |
|------|---------------|
| `crates/buzz-relay/src/conformance/mod.rs` | helpers + `EmitGuard` + `sanitized_reason_for` |
| `crates/buzz-relay/src/conformance/tracers.rs` | `NoopTracer` (prod default), `JsonlTracer` |
| `crates/buzz-relay/src/handlers/ingest.rs` | `AuthCheck`, `WriteInsert`, `WriteInsertGlobal`, `WriteDuplicate`, outer-wrapper `SanitizedError` |
| `crates/buzz-relay/src/handlers/req.rs` | **held back** — additive patch for integration onto Max's req.rs work |

## Where the checker lives

| File | What it does |
|------|--------------|
| `crates/buzz-conformance/src/lib.rs` | schema + `Tracer` trait |
| `crates/buzz-conformance/src/transitions.rs` | spec `Next` re-implementation |
| `crates/buzz-conformance/src/checker.rs` | replay engine: `IllegalTransition` / `StateMismatch` / `NonInterference` / `CoverageBreach` |

## Failure modes — what makes the gate bite

`check_trace` returns `Err(CheckError)` on any of:

- **`IllegalTransition`** — the action is not permitted from the
  current model state (e.g. `AuthCheck { verdict: Allow, claimed != resolved }`
  — M2/M8 territory).
- **`StateMismatch`** — `state_after` disagrees with the bootstrapped
  model (resolved community / bound host / actor reassigned mid-request).
- **`NonInterference`** — `row_communities` includes a label other than
  `resolved_community` (`Inv_NonInterference` / `Inv_ReadConfinement`).
- **`CoverageBreach`** — an `ImplBug` step was recorded, or a
  scenario-required action never appeared, or the trace was empty.

Each failure mode has a unit test in
`crates/buzz-conformance/src/checker.rs::tests` proving the gate bites
when you'd want it to.
