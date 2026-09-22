NIP-TW
======

Newest-first Thread Windows
---------------------------

`draft` `optional` `relay` — contract version 1

## Scope and compatibility

Opt-in extension of authenticated `POST /query`, returning the existing flat
array of signed Nostr events. No new endpoint, subscriptions, client changes,
or around-target API. Requests without `thread_window: true` retain their
existing oldest-first `depth_limit` / `thread_cursor` behavior. Channel bounds
kind **39006** and NIP-CW are unchanged. This capability does not establish the
cause of any production loading incident.

```json
{
  "thread_window": true,
  "#h": ["00000000-0000-0000-0000-000000000001"],
  "#e": ["abababababababababababababababababababababababababababababababab"],
  "kinds": [9, 40002],
  "depth_limit": 100,
  "limit": 50,
  "include_aux": true
}
```

Send as an element of the usual query filter array. Both `until` (integer Unix
seconds) and `before_id` (full 64-hex id) continue from the preceding bounds.
Both absent means newest page. Null, malformed, fractional, negative,
unrepresentable or half cursors are HTTP 400, never a restart. Hex IDs normalize
to lowercase and channel IDs to canonical UUID text.

Exactly one raw `#h` and one raw `#e` are required (duplicates are not silently
collapsed). `limit` defaults to 50, accepts 1–200; `depth_limit` defaults to 100,
accepts 1–100. Out-of-range budgets reject rather than clamp. `include_aux`
defaults to false and must be boolean when present.

`kinds` requires one to four raw entries; version 1 supports conversation row kinds
**9, 40002, 45001, 45003**. Kinds normalize to ascending, distinct integers.
Root, reactions, edits, deletions, summaries and bounds never consume the reply
budget. All other fields are rejected on an opted-in filter, including authors,
since, ids, additional tag filters, search, top_level (even false), summaries,
legacy thread cursors (both spellings), offset and page. Unsupported filters
are errors, not silently ignored constraints. Up to four windows can be served
in one query; window filters cannot be mixed with any other query mode.
This isolates their shared deadline/allowances from permissive legacy dispatch.
`thread_window: false` is inert; other non-boolean values reject.

## Selection and bounds

After fresh writer access authorization, select replies under the given root,
within the host-bound community and channel, with depth 1..depth_limit,
matching kinds and not deleted. Both metadata and event channel must match.
The root must be a conversation row kind in that community/channel; a root tombstone remains a valid
anchor. A nonexistent or wrong-channel root on an accessible channel yields a
served empty window, without expanding root aux.

The total order is:

```sql
ORDER BY tm.event_created_at DESC, tm.event_id ASC
-- continuation:
tm.event_created_at < $ts
OR (tm.event_created_at = $ts AND tm.event_id > $id)
```

Probe `limit + 1` **after** all predicates, discard the sentinel, capture the
last retained raw candidate before reconstruction. If the probe found another
row, `has_more` is true and `next_cursor` is that raw scan position. Otherwise
`has_more` is false and `next_cursor` is null. A damaged reply may be skipped;
it still consumes a scan slot and can be the cursor. Other reconstruction
errors fail the request. The sentinel never reaches the response or closure.
Neither row count nor the last delivered row is a pagination authority.

## Auxiliary closure and bounded work

When requested, expand root and retained reconstructed reply IDs (never the
sentinel): reactions 7, deletions 5/9005, edits 40003; then deletions 5/9005 of
those aux IDs. Deleted aux payloads are not delivered, but their raw IDs are
retained for deletion-of-aux discovery. Original signed events remain intact;
deduplicate by ID. Channel-less authorized deletions remain eligible.

Aux selection uses raw `limit + 1` probes and raw cursors. Failure to reconstruct
a live auxiliary event is an error, **not** a short successful page or EOF.
Thus 1,001 matches with one damaged row in the first 1,000 cannot hide the older
edit/deletion. The legacy generic aux helper is unchanged.

Budgets: 1,000 raw candidates/aux page, 200 targets/SQL query, 64 SQL aux queries
and 8,192 raw aux candidates (including tombstones and repeated cross-batch
matches and probes) across both hops and all windows per query; 8 MiB
serialized events across the query. Retries spend the same allowances;
corruption and exhausted allowances do not trigger fallback.
An 8-second deadline spans fresh access lookups, pool waits, selection,
closure, fallback, final access checks and signing for all windows in one query. New-mode SQL uses
transaction-local 4-second statement and 1-second lock timeouts, never leaked
to pooled legacy callers. Any cap/deadline/required-closure/signing failure
returns an error and no successful partial response or bounds. Query-entry
authentication retains its existing budgets.

## Signed bounds: kind 39007

Exactly one per successfully served window, including empty/exhausted pages:

```jsonc
{
  "kind": 39007,
  "pubkey": "<relay identity>",
  "tags": [
    ["d", "tw:1:<binding sha256 lowercase hex>"],
    ["h", "<canonical channel UUID>"],
    ["e", "<lowercase root id>"]
  ],
  "content": "{\"version\":1,\"direction\":\"older\",\"has_more\":true,\"next_cursor\":{\"created_at\":1751500000,\"id\":\"<64-hex id>\"}}"
}
```

Tag cardinality is exact: one d, one h, one e, nothing else. Synthesized at query
time, never persisted; client submission is rejected through shared ingest.
`next_cursor == null` iff exhausted. Bind the entire normalized response
contract with SHA-256 over UTF-8 compact JSON of this **ordered array**:

```jsonc
["tw",1,"older","<normalized host>","<reader pubkey hex>","<channel>","<root>",50,100,[9,40002],null,true]
// slots: discriminator, version, direction, host, reader, channel, root, limit, depth,
//        sorted unique kinds, request cursor, include_aux
// cursor slot: null for head, otherwise [<integer seconds>,"<lowercase id>"]
```

No whitespace, no trailing newline, ordinary decimal integers, boolean JSON
literals. The d tag is `tw:1:` followed by the lowercase digest. The host is the server-resolved
normalized request authority (see `buzz-core/src/tenant.rs`); reader is the
authenticated lowercase public key. Neither comes from filter fields. Binding
both prevents reuse across colliding tenant scopes or readers with different
aux access on the same relay. Clients must verify the **expected relay signer**
and signature, exact host/reader/scope/binding, version,
direction and cursor invariant. Missing or invalid bounds prove neither
exhaustion nor feature support. Old relays can ignore the flag and serve old
history: clients MUST NOT present that as a confirmed newest page.
A later client may explicitly fall back for an unsupported relay, but must
restart with clean **legacy** pagination state. Never reuse a descending cursor
as a legacy forward cursor, or downgrade signature/binding, authorization,
timeout, corruption or incomplete-closure errors to compatibility fallback.
This relay-only change implements no client fallback.

## Authorization and consistency

Refresh channel access from the writer for every page, before selection and
before issuing bounds; check every delivered row/aux event. Inaccessible or
nonexistent channels retain ordinary access-scoped output: no rows or bounds.
Revocation during a page cannot produce authoritative empty bounds. Access
changes affecting auxiliary channels cause a retryable error.

Cursor pages reuse the channel-window upper-bound replica proof, including
terminal pages. They do not use the forward-thread last-row/newest assumption.
The proved REPEATABLE READ replica transaction is retained through rows and
aux closure. Mid-request replica failure permanently degrades to the writer;
the bridge discards already collected aux and restarts both hops once from
the original targets, spending the same query/scan/byte/deadline allowances.
Resuming only at a pre-failure aux cursor could omit newer writer edits. Writer
follow-ups use a pool, not one globally pinned snapshot. No snapshot isolation
is promised across history pages or after degradation. Root/aux updates can
advance while a writer request runs. Existing replica deletion-lag semantics
apply: the floor proof guarantees insertion coverage, not update freshness.
Reply insertion coverage does **not** cover more recent aux timestamps or
channel-less deletions; aux is complete within the serving snapshot, not
necessarily current on the writer. Aux queries never inherit reply time bounds.
Fresh writer authorization checks are point-in-time observations, not a lease
preventing revocation after the last check.
Head routing remains the existing default-off bounded-staleness policy; this
change adds no setting and enables none.

## Index deployment

Both desired-state schema and additive migration 0049 define:

```sql
CREATE INDEX idx_thread_metadata_window
ON thread_metadata (community_id, root_event_id, event_created_at DESC, event_id ASC);
```

`thread_metadata` is not partitioned. Keep the existing indexes. Reverse
scanning this index gives ASC/DESC, not the legacy ASC/ASC; measure legacy
separately before adding another index.

For a populated production database, **before upgrading the relay**, execute
this standalone statement (not inside a transaction) through the operator's
approved schema-change workflow, coordinated with community deletion/schema
maintenance:

```sql
CREATE INDEX CONCURRENTLY idx_thread_metadata_window
ON public.thread_metadata (community_id, root_event_id, event_created_at DESC, event_id ASC);
```

Inspect `pg_index.indisvalid`, `indisready`, `indislive` and
`pg_get_indexdef(indexrelid)`. A failed concurrent build can leave an invalid
same-name index: diagnose, then drop/rebuild it concurrently before upgrading.
Do not use IF NOT EXISTS to disguise that failure. Migration 0049 validates
the exact definition and validity, even when the index already exists. Fresh
small installs create it transactionally. Startup limits lock acquisition to
one second and the build to five seconds; larger/busy installs intentionally
fail deployment rather than hold an unbounded write-blocking lock. Prebuild,
then retry. Desired-state deployments should likewise prebuild on brownfield
instances, inspect their plan and verify the catalog after apply.

Measure representative `EXPLAIN (ANALYZE, BUFFERS)` for head, deep cursor,
same-second, selective kind/depth and legacy pages; record index size and write
cost. DB spans `get_thread_window` / `thread_window_aux`, existing pool metrics,
route labels `thread_window_head` / `thread_window_cursor`, and
`buzz_thread_window_response_bytes` separate pool, SQL, aux and response costs.


### Rollback

Keep the additive index when rolling back the binary; old SQL does not need
it removed. Run old relays with `BUZZ_AUTO_MIGRATE=false` (the default): an old
embedded SQLx migrator rejects the newer migration-ledger version 49 with
`VersionMissing`. Do **not** delete ledger rows or rewrite checksums to hide
this. Roll forward to a capable migrator for future schema changes. Actual
old-binary boot/read/write verification and production-size build/write-cost
measurements remain rollout gates, not consequences proved by additive DDL.
The [local validation report](../bridge-thread-window-validation.md) records
this candidate's tests, measured costs, rollback exercise and remaining gaps.
