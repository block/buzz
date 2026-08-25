# feat(kinds): declarative event-kind registry (Seam A: scope + scoping)

Implements #3167.

## Problem

Adding a new event kind means editing several central chokepoints in
lockstep: the large `required_scope_for_kind` match in
`buzz-relay/src/handlers/ingest.rs`, two disjoint boolean allowlists
(`is_global_only_kind` / `requires_h_channel_scope`), `is_relay_only_kind`,
and the `P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` / `RESULT_GATED_KINDS` slices in
`buzz-core`. One of the invariants this spread of code has to preserve — that
`is_global_only_kind` and `requires_h_channel_scope` never both claim the same
kind — is enforced only by a runtime test, not the type system.

## What this PR does

Adds a new leaf crate, `buzz-kinds`, holding a `KindDescriptor` per event
kind: the base write-scope (`RequiredScope::Static`/`Dynamic`), the NIP-29
channel-scoping classification (`Scoping::{Global, ChannelRequired,
ChannelOptional}`), a read-gate field, and an authorship field. The relay's
`ingest_event_inner` now resolves each event's base scope and channel scoping
through `AppState.kind_registry` instead of the old match plus two
allowlists, which are deleted.

**Key decision — `Scoping` collapses the two allowlists into one enum.**
`is_global_only_kind` and `requires_h_channel_scope` were two independent
`bool` predicates that had to be kept mutually exclusive by hand; the existing
test suite carried a dedicated `global_only_and_channel_scoped_are_disjoint`
regression test for exactly this. `Scoping` makes double-classification
unrepresentable — there is no way to write a descriptor that is both
`Global` and `ChannelRequired` — so that invariant is now enforced by the
type system, not a test. (The disjointness test itself still runs, now
trivially, and I kept it rather than deleting it.)

**A registry miss is the fail-closed reject.** Every kind the old match
accepted is registered; anything else (`_ => Err("restricted: unknown event
kind")`) is simply absent from the registry, and `KindRegistry::get`
returning `None` reproduces that same reject verbatim.

**One dynamic-scope kind.** `kind:9002` (NIP-29 edit-metadata)'s scope
depends on whether the event carries an `archived` tag. This is the one
kind that needs `RequiredScope::Dynamic` plus a `KindExtension::required_scope`
hook — a minimal, single-method trait (see "What I deliberately deferred"
below for why it stops there).

## What I deliberately deferred

The upstream RFC (#3167) also names two other pieces of the same chokepoint,
which this PR does **not** touch, to keep the diff focused and reviewable:

1. **Per-kind authorize/validate residue.** The relay has inline AND-gates
   and a `validate_*` ladder (diff/engram/agent-turn-metric/event-reminder/
   persona envelope validation) that could fold into `KindExtension` as
   `authorize`/`validate` hooks. I left `KindExtension` with only
   `required_scope` in this PR — the trait can grow those methods later with
   defaults, which is non-breaking for the one existing implementor. Folding
   this in is real work against current code (some of it, e.g. the
   edit-ownership/forum-vote validators, needs richer context than the
   ingest path currently threads through) and deserves its own PR.
2. **`P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` / `RESULT_GATED_KINDS`.** These stay
   on `buzz-core`'s existing slices; `KindDescriptor::read_gate` is declared
   (so the registry already has a place to hold this once someone wires the
   read path to it) but not yet consulted anywhere. This is a mechanical,
   lower-risk follow-up once this PR's scope/scoping flip has landed and
   soaked.
3. **The `search_tsv`-NULL migration coupling.** The RFC's issue body
   describes this as coupled to `P_GATED_KINDS` via a negative allowlist
   (`P_GATED_KINDS` kinds get a NULL `search_tsv`). Current upstream has
   already moved past that: `migrations/0008_fresh_install_search_allowlist.sql`
   replaced it with a **positive** FTS allowlist (`kind IN (0, 9, 40002,
   45001, 45003) THEN to_tsvector(...) ELSE NULL`) for fresh installs, with a
   documented out-of-band maintenance path for populated databases. That's a
   different (arguably safer — fail-closed by default) design than the one
   the issue was written against, so I did not touch it; re-coupling it to
   the registry is out of scope here and would need its own discussion.
4. **`is_relay_only_kind`.** Still lives in `buzz-core` and still runs before
   the registry lookup in `ingest_event_inner`; relay-only kinds are never
   registered in `buzz-kinds` in this PR. `KindDescriptor::authorship` is
   declared for completeness but unconsumed.
5. `super::push_lease::KIND_PUSH_LEASE` (30350) keeps its inline special case
   in `ingest_event_inner` rather than being registered — it's an existing
   dedicated handler with its own scope/scoping shape, and folding it into
   `buzz-kinds` isn't necessary for this PR's behavior-preservation goal.

None of the above blocks a future PR from picking them up against this same
crate; `KindDescriptor` and `KindExtension` were designed with room for it
(see doc comments in `crates/buzz-kinds/src/descriptor.rs` and
`extension.rs`).

## How this was verified

- `cargo test -p buzz-relay --lib handlers::ingest`: **174 passed** (1 ignored,
  requires Postgres — pre-existing, unrelated to this change). These are the
  existing policy unit tests, unchanged; they now run against thin
  `#[cfg(test)]` adapters (`required_scope_for_kind` / `is_global_only_kind`
  / `requires_h_channel_scope`) that reproduce the deleted functions'
  signatures over a freshly-built registry, so the tests read identically to
  before and are the actual proof of behavior preservation.
- `cargo test -p buzz-relay --lib handlers::event`: **24 passed** (includes
  the `channel_scoped_content_kinds_require_h_tags` /
  `non_channel_kinds_do_not_require_h_tags` tests that call into the same
  adapters from a different module).
- `cargo test -p buzz-kinds`: **3 passed** — no duplicate kinds, the one
  `AUTHOR_ONLY_KINDS` cross-check against `buzz-core`, and the edit-metadata
  archived-tag scope split.
- `cargo test -p buzz-relay --lib` (full lib suite): **901 passed, 8 failed**.
  The 8 failures are pre-existing and unrelated — Postgres pool timeouts and
  two admin-endpoint status-code assertions (500 vs 404) that also fail on
  an unmodified checkout of this same commit (verified by stashing this PR's
  changes and re-running).
- `cargo check --workspace --all-targets`: clean.
- `cargo clippy -p buzz-relay -p buzz-kinds -p buzz-core --all-targets -- -D
  warnings`: clean.
- `cargo fmt --check -p buzz-relay -p buzz-kinds -p buzz-core`: clean.
- No new `unwrap()`/`expect()`/`unsafe` in production code paths — the three
  `unwrap()`s in `buzz-kinds` are inside its own `#[cfg(test)]` module, and
  `buzz-kinds/src/lib.rs` carries `#![forbid(unsafe_code)]`.

## How to test manually

```
. ./bin/activate-hermit
cargo test -p buzz-relay -p buzz-kinds -p buzz-core
just relay   # start the relay, then publish events of a few kinds via buzz-cli
             # or the desktop app and confirm scope/channel-scoping behavior
             # (e.g. a channel-scoped token can't publish kind:0/1/3; kind:9002
             # with an "archived" tag needs AdminChannels) is unchanged.
```

## Duplicate check

Re-ran the issue's "searched, none found" claim before opening this:
`gh search issues --repo block/buzz "kind registry OR required_scope_for_kind"`
and `gh search prs --repo block/buzz "3167"` / `"buzz-kinds"` / `"kind
registry"`. No open PR implements #3167 as of this writing. Two related open
RFCs by the same author exist — #3351 (agent-context provider registry in
`buzz-acp`) and #3280 (desktop channel-feature registry) — both explicitly
describe themselves as companions to #3167, not overlapping implementations
of it.
