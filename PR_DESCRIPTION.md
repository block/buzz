# feat(kinds): declarative event-kind registry (scope + scoping)

Implements #3167.

## Problem

Adding a new event kind means editing several places at once: the large
`required_scope_for_kind` match in `buzz-relay/src/handlers/ingest.rs`, two
separate boolean allowlists (`is_global_only_kind` / `requires_h_channel_scope`),
`is_relay_only_kind`, and the `P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` /
`RESULT_GATED_KINDS` slices in `buzz-core`. One invariant this code has to
hold, that `is_global_only_kind` and `requires_h_channel_scope` never both
claim the same kind, is only checked by a runtime test today, not by the
type system.

## What this PR does

Adds a new leaf crate, `buzz-kinds`, holding a `KindDescriptor` per event
kind: the base write scope (`RequiredScope::Static`/`Dynamic`), the NIP-29
channel-scoping classification (`Scoping::{Global, ChannelRequired,
ChannelOptional}`), a read-gate field, and an authorship field.
`ingest_event_inner` now resolves each event's scope and channel scoping
through `AppState.kind_registry` instead of the old match plus the two
allowlists, which are removed.

**`Scoping` replaces the two allowlists with one enum.** The old
`is_global_only_kind` and `requires_h_channel_scope` were two independent
bools that had to be kept mutually exclusive by hand, backed by a dedicated
`global_only_and_channel_scoped_are_disjoint` regression test. With
`Scoping`, a descriptor can't be both `Global` and `ChannelRequired` at the
same time, so that invariant is now structural, not test-only. I kept the
disjointness test anyway, it just passes trivially now.

**A registry miss is the fail-closed reject.** Every kind the old match
accepted is registered. Anything else falls through to the same
`"restricted: unknown event kind"` error as before.

**One dynamic-scope kind.** `kind:9002` (NIP-29 edit-metadata) needs
`RequiredScope::Dynamic` plus a `KindExtension::required_scope` hook,
because its scope depends on whether the event carries an `archived` tag.

## Not in this PR

The RFC (#3167) also names two more pieces of the same problem. I left both
out to keep this diff reviewable:

1. **Per-kind authorize/validate logic.** The relay has inline AND-gates
   and a `validate_*` ladder (diff/engram/agent-turn-metric/event-reminder/
   persona envelope) that could become `KindExtension::authorize`/`validate`
   hooks. `KindExtension` only has `required_scope` in this PR. The trait
   can grow those methods later with defaults, which won't break the one
   existing implementor. Some of that logic (edit-ownership and forum-vote
   validation, for example) needs more context than the ingest path
   currently threads through, so it deserves its own PR.
2. **`P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` / `RESULT_GATED_KINDS`.** These
   still live on `buzz-core`'s existing slices. `KindDescriptor::read_gate`
   is declared but nothing reads it yet. This is a mechanical follow-up
   once the scope/scoping flip has landed and soaked.
3. **The `search_tsv`-NULL migration.** The RFC issue describes this as
   coupled to `P_GATED_KINDS` through a negative allowlist. That's no
   longer how it works: `migrations/0008_fresh_install_search_allowlist.sql`
   already replaced it with a positive FTS allowlist for fresh installs.
   That's a different, arguably safer design than the one the issue was
   written against, so I left it alone.
4. **`is_relay_only_kind`.** Still lives in `buzz-core` and still runs
   before the registry lookup. Relay-only kinds aren't registered in
   `buzz-kinds` in this PR. `KindDescriptor::authorship` is declared but
   unused for now.
5. `KIND_PUSH_LEASE` (30350) keeps its existing inline special case in
   `ingest_event_inner` rather than being registered. It's a dedicated
   handler with its own shape, and folding it in isn't needed for this
   PR's behavior-preservation goal.

None of this blocks picking it up later. `KindDescriptor` and
`KindExtension` were built with room for it, see the doc comments in
`crates/buzz-kinds/src/descriptor.rs` and `extension.rs`.

## Review focus

1. Is deferring authorize/validate logic into `KindExtension` acceptable as
   its own follow-up PR, or should more of it be bundled here given it's
   part of the same chokepoint?
2. `KindDescriptor::read_gate` is declared but not yet consumed anywhere.
   Is that an acceptable staging point, or should the read-gate flip land
   in this same PR since it's mechanical and low-risk?
3. Any objection to leaving `is_relay_only_kind` and `KIND_PUSH_LEASE`'s
   inline handling outside the registry for now, as described above?

## Testing

At commit `a05a9c51b` (the last code commit on this branch; later commits
only touch this description):

- `cargo test -p buzz-relay --lib handlers::ingest`: 174 passed, 1 ignored
  (needs Postgres, pre-existing, unrelated). These are the existing policy
  tests, unchanged, now running against `#[cfg(test)]` adapters that
  reproduce the deleted functions' signatures over a freshly built
  registry. That's the actual proof this is behavior-preserving.
- `cargo test -p buzz-relay --lib handlers::event`: 24 passed.
- `cargo test -p buzz-kinds`: 3 passed (no duplicate kinds, the
  `AUTHOR_ONLY_KINDS` cross-check against `buzz-core`, the edit-metadata
  archived-tag scope split).
- `cargo test -p buzz-relay --lib` (full): 901 passed, 8 failed. The 8
  failures are pre-existing and unrelated (Postgres pool timeouts, two
  admin-endpoint status-code assertions). I confirmed they also fail on an
  unmodified checkout of the same commit.
- `cargo check --workspace --all-targets`: clean.
- `cargo clippy -p buzz-relay -p buzz-kinds -p buzz-core --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p buzz-relay -p buzz-kinds -p buzz-core`: clean.
- No new `unwrap()`/`expect()`/`unsafe` in production code. The `unwrap()`s
  in `buzz-kinds` are in its own test module, and `buzz-kinds/src/lib.rs`
  has `#![forbid(unsafe_code)]`.

### Manual test

```
. ./bin/activate-hermit
cargo test -p buzz-relay -p buzz-kinds -p buzz-core
just relay
```

Then publish events of a few kinds through buzz-cli or the desktop app and
confirm scope/channel-scoping behavior is unchanged: a channel-scoped token
still can't publish kind:0/1/3, and kind:9002 with an `archived` tag still
needs AdminChannels.

## Duplicate check

Re-ran the issue's "searched, none found" claim before opening this:
`gh search issues --repo block/buzz "kind registry OR required_scope_for_kind"`
and `gh search prs --repo block/buzz "3167"` / `"buzz-kinds"` / `"kind registry"`.
No open PR implements #3167. Two related open RFCs from the same author
exist, #3351 (agent-context provider registry) and #3280 (desktop
channel-feature registry), both explicitly companions to #3167, not
overlapping implementations.
