NIP-TW
======

Thread Window
-------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (events and filters), NIP-29 (groups), NIP-98 (HTTP auth)

## Abstract

A thread window is a newest-first page of replies served through the existing
NIP-98-authenticated `POST /query`. The response is a flat array of signed events:
reply rows, optional auxiliary events, and one relay-signed `kind:39007` bounds
overlay. No endpoint, subscription, or around-target operation is added.

Absent or false `thread_window` preserves the existing oldest-first
`depth_limit` / `thread_cursor` behavior. [NIP-CW](NIP-CW.md) and its channel
bounds (`kind:39006`) are unchanged.

## Request

Submit the filter in the usual query filter array:

```jsonc
{
  "thread_window": true,
  "#h": ["<channel UUID>"],
  "#e": ["<root event id>"],
  "kinds": [9, 40002],
  "depth_limit": 100,
  "limit": 50,
  "include_aux": true,
  "until": 1751500000,         // continuation: both cursor fields or neither
  "before_id": "<64-hex id>"
}
```

- `#h` and `#e` require exactly one raw entry each. UUIDs and full 64-hex IDs
  normalize to canonical lowercase text; duplicate entries are not collapsed.
- `kinds` requires 1–4 entries from conversation kinds 9, 40002, 45001, 45003;
  it normalizes to sorted, distinct integers.
- `limit` defaults to 50 (range 1–200); `depth_limit` defaults to 100 (1–100).
  Out-of-range values reject rather than clamp. `include_aux` defaults to false
  and must be boolean.
- `until` is a nonnegative integer Unix timestamp representable by the relay;
  `before_id` is a full 64-hex ID. Both absent selects the head. Half, null,
  malformed, fractional, negative or out-of-range cursors reject, never restart.
- Every other field rejects, including legacy cursor spellings, `top_level`
  (even false), search, summaries, authors, additional tags, `since`, IDs,
  offsets and page numbers. Non-boolean `thread_window` also rejects.

Invalid requests return HTTP 400. A query accepts at most four window filters
and cannot mix them with other query modes.

## Relay processing

1. Refresh channel access from the writer. Inaccessible or nonexistent channels
   return no rows or bounds. Within the host-derived community, require a
   conversation-kind root in the requested channel. A root tombstone remains
   valid; a missing or wrong-channel root serves an empty window without root aux.
2. Select non-deleted replies at depths 1..`depth_limit`, matching `kinds`, with
   both event and metadata channel equal to `#h`. Order by `created_at DESC,
   id ASC`. Continuation keeps `created_at < until OR (created_at = until AND
   id > before_id)`.
3. Probe `limit + 1` after **all** predicates. Discard the sentinel before
   response/aux processing. If it exists, set `has_more: true` and take
   `next_cursor` from the last retained raw candidate, before reconstruction.
   Otherwise return `has_more: false, next_cursor: null`. A damaged reply can
   consume a slot and become the cursor without being delivered; other
   reconstruction errors fail the request.
4. If requested, expand the root and retained reconstructed replies: reactions
   (7), deletions (5/9005), edits (40003), then deletions of those aux IDs.
   Preserve original signatures and deduplicate by ID. Apply reader access to
   every event, including channel-less deletions. Deleted aux payloads are
   omitted but their IDs remain targets for the second hop. Drain each hop
   with raw cursors; a damaged live aux event fails rather than implying EOF.
5. Refresh writer access before signing. Revoked access to the requested
   channel returns no rows or bounds; any other access-set change returns a
   retryable error. Append exactly one bounds event per served window, including
   empty and exhausted windows. Root, aux and bounds do not consume `limit`.

## Bounds: kind 39007

Bounds are query-time metadata, never stored. Client submissions MUST be
rejected at ingest. Tags are exactly one `d`, one `h`, one `e`:

```jsonc
{
  "kind": 39007,
  "pubkey": "<relay identity>",
  "tags": [
    ["d", "tw:1:<binding SHA-256 lowercase hex>"],
    ["h", "<canonical channel UUID>"],
    ["e", "<lowercase root id>"]
  ],
  "content": "{\"version\":1,\"direction\":\"older\",\"has_more\":true,\"next_cursor\":{\"created_at\":1751500000,\"id\":\"<64-hex id>\"}}"
}
```

The binding hashes UTF-8 compact JSON of this ordered array, without whitespace
or a trailing newline, using decimal integers and JSON booleans:

```jsonc
["tw",1,"older","<host>","<reader hex>","<channel>","<root>",50,100,[9,40002],null,true]
// slots: discriminator, version, direction, host, reader, channel, root,
//        limit, depth, sorted unique kinds, request cursor, include_aux
// cursor: null for head, otherwise [<seconds>,"<lowercase id>"]
```

Host is the server-resolved normalized authority (`buzz-core/src/tenant.rs`);
reader is the authenticated lowercase pubkey. Neither comes from filter fields.

Clients MUST verify the expected relay signer and signature, exact tags and
request binding, version, direction, and `next_cursor == null` iff exhausted.
Only validated bounds determine exhaustion; row count and the last delivered
row do not. Echo `next_cursor` as `until` / `before_id` to continue.

An old relay may ignore the flag and return oldest-first history without
bounds. Missing/invalid bounds prove neither exhaustion nor support. An
explicit compatibility fallback must restart with clean legacy state, never
reuse a descending cursor. Signature/binding, authorization, timeout, corruption
and incomplete-closure failures MUST NOT trigger compatibility fallback.
This extension implements no client opt-in or fallback.

## Consistency and limits

Cursor pages use NIP-CW's upper-bound replica proof, including terminal pages,
retaining the proved REPEATABLE READ transaction through rows and aux. Reader
failure degrades permanently to the writer and restarts both aux hops once
from the original targets, discarding collected aux but retaining spent budgets.
Writer follow-ups use a pool, not a pinned snapshot; no cross-page snapshot is
promised. Reply insertion coverage does not prove freshness of edits, deletions
or newer aux. Aux is complete within the serving snapshot and never inherits
reply time bounds. Writer authorization is a point-in-time check, not a lease.
Head routing keeps the existing default-off bounded-staleness policy.

Limits are shared across all windows and retries in one query:

| Resource | Limit |
|---|---|
| Aux scan | 1,000 raw candidates/page; 200 targets/SQL query |
| Aux work | 64 SQL queries; 8,192 raw candidates, including tombstones, repeated matches and probes |
| Serialized response | 8 MiB |
| Whole request after authentication | 8 seconds, including access, pool waits, fallback and signing |
| Selection/aux transaction | 4-second statement timeout; 1-second lock timeout |

Authorization uses ordinary writer-pool budgets (defaults: 5-second lock and
3-second acquisition timeout). Pool, statement, lock and outer timeouts return
retryable HTTP 503; no exact wait is promised. Exhausted budgets, required
closure or signing failures return an error without partial rows or bounds.
Corruption and exhausted budgets do not trigger replica fallback. Transaction
settings do not leak to legacy callers; query-entry authentication is unchanged.

## Deployment

Desired-state schema and additive migration 0048 add an index on the
unpartitioned `thread_metadata`. On populated databases, prebuild it through
the approved schema-change workflow before upgrading, outside a transaction
and coordinated with community deletion/schema maintenance:

```sql
CREATE INDEX CONCURRENTLY idx_thread_metadata_window
ON public.thread_metadata (community_id, root_event_id, event_created_at DESC, event_id ASC);
```

Verify `pg_get_indexdef`, `indisvalid`, `indisready`, and `indislive`. Diagnose
and rebuild failed same-name indexes; do not hide them with `IF NOT EXISTS`.
Migration 0048 validates the definition and skips CREATE for a valid prebuild
(even `CREATE INDEX IF NOT EXISTS` takes a writer-conflicting lock). Fresh
installs build transactionally with 1-second lock/5-second statement limits;
busy or larger installs must prebuild and retry. Inspect desired-state plans
and verify the catalog after apply too.

Keep existing indexes. Reverse scanning this index is ASC/DESC, not legacy
ASC/ASC. Before rollout, measure representative head, deep, same-second,
selective kind/depth and legacy plans, index size and write cost; validate the
actual replica topology. Spans `get_thread_window` / `thread_window_aux`, route
labels `thread_window_head` / `thread_window_cursor`, pool metrics and
`buzz_thread_window_response_bytes` expose query and response costs.

For binary rollback, keep the index and set `BUZZ_AUTO_MIGRATE=false` (default).
Old embedded SQLx migrators reject ledger version 48 with `VersionMissing`.
Never delete ledger rows or rewrite checksums; roll forward for schema changes.
Verify old-binary boot/read/write on the upgraded schema before deployment.
