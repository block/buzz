# Task 2 — strict calendar contracts and signed relay persistence

## Delivered

- Added immutable, fail-closed TypeScript contracts for sources, events,
  revisions, and revision chunks. Parsers require exact keys, RFC3339-style
  offset timestamps, ordered time windows, strict ownership, bounded text, and
  no more than 2,000 imported changes; parsed objects and arrays are frozen.
- Added the matching Rust serde wire records and a JSON v1 fixture consumed by
  both TypeScript and `buzz-core` tests, so the source/event field spelling is
  shared rather than copied.
- Added signed Nostr codecs: NIP-33 source/event heads use stable `d` tags;
  event tags carry start/end/source/revision; source carries source/revision and
  coverage; regular kind 46310 revisions are immutable chunks with revision,
  source, index/count, and full-manifest SHA-256 tags.
- Added relay-only fetch and mutation services plus React Query hooks. Reads
  filter explicitly by kind and owner, select newest `(kind,d)`, fail closed on
  malformed events, and return only range-overlapping event heads.
- Re-import writes are deliberately ordered: source-owned event heads, all
  revision chunks, then the source active-revision pointer. Source/revision
  mismatch or a manual/other-source event aborts before the first publish.
  Chunk construction/validation completes before writing, so an unsuccessful
  chunk phase cannot select the new pointer.

## TDD evidence

1. Wrote parser and Rust fixture tests before their modules; the desktop test
   initially failed with `ERR_MODULE_NOT_FOUND` for `contracts.ts`.
2. Wrote codec/service tests before their modules; both initially failed with
   `ERR_MODULE_NOT_FOUND` for `eventCodec.ts` and `battleRhythmService.ts`.
3. Added the smallest implementations until the focused tests were green,
   then formatted and repeated the checks.

## Verification

- `cargo fmt --check` — passed.
- `cargo test -p buzz-core --test battle_rhythm_contracts` — passed (1 test).
- Focused desktop contract/codec/service tests — passed (8 tests).
- `pnpm typecheck` — passed.
- `pnpm test -- src/features/battle-rhythm` executes the repository-wide test
  glob successfully but then Node treats the trailing directory argument as a
  test target; it reports one harness failure at `src/features/battle-rhythm`.
  This is the existing script/argument behavior, not a Battle Rhythm assertion
  failure; the focused invocation above verifies every Task 2 test.

## Self-review

- No route, UI, importer, Apple/EventKit, standalone database, or external
  service changes were made.
- Manual events are rejected by import mutations; event heads from other
  sources/revisions are rejected before publication.
- The immutable revision payload itself is manifest-hashed and every finalized
  serialized chunk is bounded at 240 KiB before signing.
