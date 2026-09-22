NIP-CW
======

Channel and Thread Windows
--------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format, filters), NIP-11 (relay information document), NIP-29 (relay-based groups), NIP-98 (HTTP auth)

## Abstract

This NIP defines two relay-computed, cursor-paged views served as ordinary signed Nostr events through extended NIP-01 filters:

- **channel mode** pages a channel's top-level timeline;
- **thread mode** pages one root's descendants newest-first.

Both modes use stable keyset order and explicit relay-signed exhaustion bounds. A channel-mode request may also include the aux closure and thread-summary overlays:

- the **aux closure** — stored reactions, deletions, and edits targeting the returned rows, with their original authors and signatures (`include_aux`),
- **thread summaries** — one relay-signed `kind:39005` per row that has replies (`include_summaries`),
- **channel bounds** — exactly one relay-signed `kind:39006` carrying the authoritative `has_more` fact and the next-page cursor.

Thread mode returns reply rows, optional bounded aux closure, and exactly one request-bound `kind:39007` thread-bounds overlay. `39006` and `39007` are deliberately distinct: the existing channel/cursor identity of `39006` cannot disambiguate concurrent roots and MUST NOT be reinterpreted.

The extension adds no endpoint and no envelope. The wire format is the flat array of signed events the query surface already returns; a client that ignores this NIP receives standard behavior everywhere.

## Motivation

A NIP-01 filter can only *match* tag values; it cannot express their absence. "Channel messages that are **not** replies" — the timeline every threaded-chat client renders first — is therefore inexpressible in vanilla filters, so generic clients page the full event stream and reassemble threads client-side. That costs bandwidth proportional to reply volume, and worse, it breaks pagination correctness: `limit` counts raw events, so a page of 50 events may contain 3 top-level rows or 50, and the client cannot ask for "the next 50 rows."

Timestamp pagination (`until` alone) has a second defect: `created_at` has one-second resolution, so bursts of same-second events make a timestamp cursor lossy or duplicative at every page boundary.

A relay that computes thread structure at ingest already knows which events are top-level. This NIP lets a client request that view directly, with a composite `(created_at, id)` cursor that is exact under same-second bursts, and with server-computed exhaustion (`has_more`) so an exact-multiple final page is not misread as "more available."

## Non-Goals

This NIP does not change ingest, storage, or fan-out. Rows returned in a window are ordinary stored events; the overlays are computed per query and never stored.

This NIP does not define around-target retrieval or cross-page snapshot isolation. It defines protocol compatibility and fallback rules, but the Buzz thread-mode implementation ships no client opt-in or fallback implementation. Thread mode starts at the newest reply and continues toward older replies.

This NIP does not require WebSocket REQ support. A relay MAY serve window filters only on an HTTP query surface and ignore the extension fields on REQ (see §Degradation).

## Terminology

This document uses MUST, MUST NOT, SHOULD, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The keypair whose pubkey the relay advertises (e.g. NIP-11 `self`). All overlay events are signed with it.
- **row**: A stored, signed event returned as part of the page proper (usually client-authored; Buzz also stores relay-signed events carrying actor provenance). Rows are the only events that count against `limit`.
- **top-level**: An event that opens a thread rather than replying into one — defined by wire tags in §Channel-mode Top-level Classification.
- **overlay**: A relay-signed event (`kind:39005`, `kind:39006`, or `kind:39007`) synthesized at query time. Overlays are metadata *about* rows: never a row, never a cursor input, never durable history.
- **composite cursor**: The pair `(created_at, id)` identifying a position in the total order. `created_at` is unix seconds; `id` is a 64-character lowercase hex event id.
- **scan position**: The composite cursor of the last event the relay's query *retained*, whether or not that event was ultimately delivered as a row (see §Channel-mode Relay Processing Algorithm step 3). The cursor tracks where the scan stopped, not what the client received.

## Channel-mode Request

A channel-mode window request is a standard filter plus extension fields, submitted wherever the relay accepts filters (for Buzz: the NIP-98-authenticated HTTP bridge `POST /query`):

```jsonc
{
  "kinds": [9],                  // optional row-kind restriction
  "#h": ["<channel-id>"],        // REQUIRED: exactly one channel
  "limit": 50,                   // row budget (rows only, never overlays)
  "top_level": true,             // selects the window path
  "include_summaries": true,     // optional: kind:39005 overlays
  "include_aux": true,           // optional: aux closure
  "until": 1751500000,           // ┐ composite request cursor —
  "before_id": "<64-hex id>"     // ┘ both or neither
}
```

- `top_level` — MUST be boolean `true` to select the window path. Any other value (absent, `false`, string, number) means the filter is served as a normal filter.
- `#h` — the window MUST target exactly one channel. Zero or multiple channels: reject with an error (Buzz: HTTP `400`). A channel the requester cannot access is handled by §Channel-mode Access Scoping, not by an error that confirms the channel exists.
- `limit` — the row budget. Overlays and aux events MUST NOT count against it. Relays SHOULD clamp it to a documented range (Buzz: default 50, maximum 200, minimum 1).
- `until` + `before_id` — the request cursor: the `next_cursor` from the previous page's `kind:39006` overlay, echoed verbatim — `until` = `next_cursor.created_at`, `before_id` = `next_cursor.id`. **Both present or both absent.** Exactly one present MUST be rejected: a timestamp-only cursor silently loses or duplicates same-second rows, which is the failure mode this NIP exists to remove. Both absent = head-of-channel request.
- `kinds` — optional; restricts which kinds may be rows. It does not affect overlay or aux kinds.

Cursor grammar: `until` MUST be a non-negative integer of unix seconds representable by the relay's timestamp type; `before_id` MUST be exactly 64 hexadecimal characters (the lowercase form emitted in `next_cursor.id` is canonical). A malformed value MUST cause rejection of the request — it MUST NOT be ignored and demoted to a half cursor or a head request.

Offset/page-number pagination MUST NOT be honored on the window path.

## Channel-mode Top-level Classification

The row set must be reproducible from wire data alone, so the reply/top-level distinction is defined by tags, not by any relay's storage schema.

An event is a **reply** iff it carries a NIP-10 *marked* `e` tag with the `reply` marker (`["e", "<parent-id>", <relay-url>, "reply"]`, parent id being 64 hex characters). An event with no marked `reply` e-tag — including one carrying only a `root`-marked tag, unmarked/positional e-tags, or no e-tags at all — is **not** a reply.

From that predicate:

- **depth** 0 = not a reply. A reply's depth is its parent's depth + 1, following `reply` markers up the ancestry (relays MAY cap depth; Buzz rejects beyond 100). A reply MUST target a parent in the same channel; its `root` marker, when present, MUST agree with the parent's ancestry.
- **broadcast**: a reply is *broadcast to the channel* iff it carries the exact tag `["broadcast", "1"]`. Broadcasting is an author's opt-in to surface a depth-1 reply on the channel timeline as well as in its thread.

An event is **top-level** — eligible to be a window row — iff its depth is 0, or its depth is 1 and it is broadcast.

Storage fallback (fail-open): a relay that indexes this classification at ingest may hold events stored before the index existed, whose depth is unknown. Such events MUST be treated as top-level rather than vanishing from every window. This is a compatibility rule for pre-index data, not a third protocol state — an interoperating implementation classifying from tags alone has no unknown case.

## Channel-mode Relay Processing Algorithm

For a valid window filter on an accessible channel (§Channel-mode Access Scoping) the relay MUST:

1. **Select rows.** From the target channel, take events that are top-level (§Channel-mode Top-level Classification), not deleted, and matching `kinds` if present, in the total order `created_at DESC, id ASC` (`id` compared bytewise). With a cursor `(ts, id)`, retain only events where `created_at < ts OR (created_at = ts AND id > id)`.
2. **Probe exhaustion.** Evaluate the query with an internal budget of `limit + 1` rows *after all predicates*. If `limit + 1` rows match, `has_more = true` and the sentinel row is discarded — it MUST NOT appear on the wire, in overlays, or in the aux closure. Otherwise `has_more = false`.
3. **Derive the next cursor.** If `has_more`, `next_cursor` is the **scan position**: the composite cursor of the last retained candidate, captured *before* any serving-time reconstruction or filtering of individual events. Otherwise `next_cursor = null`. The invariant `next_cursor = null ⇔ has_more = false` MUST hold. Because it is a scan position, `next_cursor` MAY reference an event that does not appear in the response (e.g. one skipped by the relay as unreconstructable); it is authoritative regardless, and deriving it from delivered rows instead would stall pagination on every skipped event.
4. **Append the aux closure** (if `include_aux` and at least one row): two hops of events referencing the rows by `e` tag. Hop 1: reactions (`kind:7`), deletions (`kind:5`, `kind:9005`), and edits (Buzz `kind:40003`) whose `e` tag is a row id. Hop 2: deletions whose `e` tag is a hop-1 event id (a delete-of-a-reaction). Each event appears at most once; access-scoped events the requester cannot read are omitted. Relays MAY cap each hop (Buzz: 1000 events per hop).
5. **Append thread summaries** (if `include_summaries`): one `kind:39005` per row that has at least one reply. Rows without replies get none.
6. **Append window bounds**: exactly one `kind:39006` per served window response, always — including empty and exhausted pages.

The response is the surface's ordinary flat array of signed events — rows first in keyset order, then aux, then summaries, then bounds. Clients MUST partition by kind and MUST NOT rely on array position beyond the ordering of rows.

## Channel-mode Access Scoping

Access is evaluated before any of the steps above. A syntactically valid window request for a channel the requester cannot access — including a channel that does not exist — MUST produce the relay's ordinary access-scoped result for that surface, with **no rows and no overlays**. For Buzz's query surface that ordinary result is an empty array, exactly as any other filter against an inaccessible channel produces.

Two consequences implementers MUST NOT miss:

- The "exactly one `kind:39006`" guarantee applies only to *served* windows — responses where access succeeded. The absence of a bounds overlay is therefore meaningful: it tells an extension-aware client that no window was served (access-scoped, or the relay does not implement this NIP — see §Degradation).
- An inaccessible channel is thereby indistinguishable from a nonexistent one, but *not* from an accessible empty channel: the latter is a served window and does return a `39006` (`has_more: false`). This is the same existence-disclosure posture as the relay's ordinary reads — a requester who can query a channel at all was already entitled to know it exists.

## Channel-mode Overlay Event Formats

Overlays are signed by the relay identity and synthesized per response. Both kinds sit in the parameterized-replaceable range, so a client that caches them gets replace-by-`d`-tag semantics from NIP-01 with no special handling. Relays MUST reject client-submitted events of either kind at ingest.

### `kind:39005` — thread summary

One per returned row with replies. Tag cardinality is exact: one `e`, one `d`, one `h`, nothing else.

```jsonc
{
  "kind": 39005,
  "pubkey": "<relay-identity-pubkey>",
  "tags": [
    ["e", "<row-event-id>"],
    ["d", "<row-event-id>"],
    ["h", "<channel-id>"]
  ],
  "content": "{\"reply_count\":4,\"descendant_count\":7,\"last_reply_at\":1751500123,\"participants\":[\"<hex-pubkey>\",\"...\"]}"
}
```

- `reply_count` — direct replies to the row. `descendant_count` — all events in the row's thread subtree.
- `last_reply_at` — unix seconds of the newest descendant, or `null`.
- `participants` — up to 10 distinct author pubkeys from the thread, most recent first.
- The `e` and `d` tags both carry the row's event id: `e` for reference-following, `d` for replaceable addressing.

### `kind:39006` — window bounds

Exactly one per served window response. The **only** authority on exhaustion. Tag cardinality is exact: one `d`, one `h`, nothing else.

```jsonc
{
  "kind": 39006,
  "pubkey": "<relay-identity-pubkey>",
  "tags": [
    ["d", "<channel-id>:<request-cursor-or-head>"],
    ["h", "<channel-id>"]
  ],
  "content": "{\"has_more\":true,\"next_cursor\":{\"created_at\":1751499000,\"id\":\"<64-hex id>\"}}"
}
```

- `d`-tag suffix (canonical serialization): the literal string `head` for a head request, else `<created_at>:<event_id>` — decimal unix seconds, colon, full 64-character lowercase hex id — identifying the *request* cursor this page answered. Clients MUST verify the suffix equals the cursor they sent and discard the overlay (and the page) on mismatch; this binds each bounds overlay to its request and makes concurrent-page responses unambiguous.
- `next_cursor` — the composite cursor to echo as `until` + `before_id` for the next page, or `null` iff `has_more` is `false`.
- Reserved: an `oldest_retained` content field may be added (retention gap signaling) without a wire break. Clients MUST ignore unknown content fields.

## Channel-mode Client Behavior

1. **Head request**: send the window filter with no cursor. Render rows in received order.
2. **Continue**: read `kind:39006`; if `has_more`, send the same filter with `until = next_cursor.created_at`, `before_id = next_cursor.id`. Repeat until `has_more = false`.
3. **Exhaustion**: `39006.has_more` is the only exhaustion signal. `rows < limit` proves nothing — an exact-multiple final page returns `limit` rows with `has_more = false`, and predicate filtering can shrink any page. A client MUST NOT stop paging on row count, and MUST NOT treat a full page as "more available."
4. **Immutability**: fetched pages are immutable history chained cursor→cursor. New live events MUST NOT be spliced into fetched pages; deliver them through a separate live subscription (`since: now`) and merge at render time. On reconnect, refetch the head page and re-arm the live subscription; deeper pages need no repair.
5. **Bounds integrity**: a window response missing its `kind:39006`, or carrying more than one, or carrying one whose `d`-tag binding does not echo the request cursor, whose content is not parseable JSON, or whose content violates `has_more = true ⇔ next_cursor ≠ null`, is not a usable page — the client MUST discard it (and MAY retry) rather than guess at exhaustion. Clients SHOULD additionally reject overlays that violate the exact tag cardinality of §Channel-mode Overlay Event Formats or whose content fields have the wrong runtime types (hardening against a malformed or hostile serializer). Cryptographic verification is governed by §Overlay Trust.
6. **Overlays are metadata**: never render a `39005`/`39006` as a message, never feed one into cursor math, and key cached summaries by their `d` tag (latest wins).

## Thread Mode

`thread_window: true` selects thread mode. It is an additive selector within this NIP, not a reinterpretation of channel mode. `top_level: true` and `kind:39006` retain their existing channel semantics; absent or false `thread_window` retains the legacy oldest-first thread path.

A thread window is a newest-first page of replies served through the existing
NIP-98-authenticated `POST /query`. The response is a flat array of signed events:
reply rows, optional auxiliary events, and one relay-signed `kind:39007` bounds
overlay. No endpoint, subscription, or around-target operation is added.

Absent or false `thread_window` preserves the existing oldest-first
`depth_limit` / `thread_cursor` behavior. Channel mode and its channel bounds (`kind:39006`) are unchanged.

### Request

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

### Relay processing

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

### Thread Bounds: kind 39007

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
The Buzz thread-mode implementation ships no client opt-in or fallback implementation.

### Consistency and limits

Thread cursor pages use channel mode's upper-bound replica proof, including terminal pages,
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

### Deployment

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

Migration 0048 retains its original NIP-TW wording to preserve its SQLx checksum; those deployment references now refer to this section.

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

## Degradation

The channel-mode extension fields in this NIP are *additional* keys on a standard filter, and clients and relays that do not implement it need no changes:

- **Extension-unaware relay**: a tolerant filter parser (one that ignores unknown keys, as common NIP-01 implementations do) serves the filter as a plain `kinds` + `#h` query — a complete, correct, standard event stream. A strict parser may instead reject the filter outright. Both are safe: neither produces a wrong-but-plausible top-level timeline. A client MUST treat *either* signal — a response with no valid `kind:39006`, or an error/unsupported-filter response — as a downgrade, and fall back by reissuing a clean standard filter with all extension keys removed and assembling threads client-side. (Buzz's own WebSocket REQ path is such a tolerant parser: the filter deserializer drops the extension fields, so a window filter on REQ serves the standard query.)
- **Extension-unaware client**: never sends `top_level`, never sees an overlay kind, and observes a completely standard relay.

A relay implementing this NIP MAY advertise it in its NIP-11 relay information document; the discovery mechanism is out of scope for this NIP. A client needs no advertisement to probe safely: send one head window request and apply the downgrade rule above — the presence of a valid `kind:39006` is the capability signal.

## Security and Privacy Considerations

Overlays are relay-authored facts about data the requester can already read. A relay MUST apply the applicable mode's access rules to rows and to every aux-closure event. Inaccessible channels produce no rows, no overlays, and no distinguishable existence error; thread mode additionally validates the requested root within the host-derived community and channel before serving descendants.

`kind:39005` aggregates thread activity (participant pubkeys, counts, recency) into one event. It only ever describes threads rooted in a channel the requester can read, so it reveals nothing a client could not compute from readable events — it saves round trips, not permissions.

Client-submitted `39005`/`39006`/`39007` events MUST be rejected at ingest (relay-only kinds); a forged overlay accepted into storage could later masquerade as relay-signed state.

### Overlay Trust

`kind:39006` and `kind:39007` are the pagination authorities for channel and thread mode respectively. Thread-mode clients MUST perform the signer, signature, tag, and request-binding verification specified in §Thread Bounds. Before using the channel-window fast path, a client MUST adopt exactly one of these trust profiles:

- **Authenticated-transport profile** (what Buzz desktop ships): the client speaks to a relay it deliberately configured as its source of truth, over TLS (HTTPS/WSS) to that configured origin — server-origin authentication comes from the TLS certificate chain, which is what proves the response bytes came from the relay. (NIP-98 request signing and NIP-42 auth run over this channel too, but they authenticate the *requester* to the relay for access control; they are not evidence of response provenance.) The MUST-level structural checks of §Channel-mode Client Behavior step 5 — exactly one bounds, request binding, parseable content, `has_more`/`next_cursor` agreement — are still mandatory and are what #1500 enforces. The SHOULD-level checks of step 5 (exact tag cardinality, runtime field-type validation) and cryptographically binding channel overlay signatures to the advertised NIP-11 identity are future hardening, to be applied uniformly across relay-signed reads (with NIP-DV, NIP-IA), not a current channel-mode guarantee. Under this profile, "relay-signed" is a TLS-origin claim, not a client-verified cryptographic one.
- **Identity-verified profile**: the client has obtained and trusts the relay identity pubkey out-of-band or via NIP-11. It MUST verify each overlay's event id, Schnorr signature, and signer against that identity, and treat any failure as the §Channel-mode Client Behavior step-5 discard. This is the profile for clients that cannot or do not authenticate their transport end-to-end.

A channel-mode client with neither an authenticated transport nor a verifiable relay identity MUST NOT use the channel-window fast path: it falls back to the standard filter (§Degradation), where it verifies every event signature itself. Thread mode has the stricter verification and clean-legacy-restart rules in §Thread Bounds; it MUST NOT downgrade on an invalid signed response.

## Implementation Gotchas

- The `limit + 1` probe MUST run after *all* predicates: access, deletion and `kinds` in both modes; top-level classification in channel mode only; root, channel and depth restrictions in thread mode. A probe over a superset produces false `has_more = true` on the last page.
- The cursor comparison uses `id > $id` (bytewise ascending) because the total order is `created_at DESC, id ASC`. Getting the id inequality backwards drops or duplicates same-second rows — precisely the bug the composite cursor removes.
- `next_cursor` is the last retained *scan candidate*, not the last delivered row: capture the scan position before per-event reconstruction so a skipped event cannot stall pagination. Clients echo it verbatim and never derive or validate it against the rows they received.
- **Channel mode only:** events ingested before the relay computed thread metadata have no depth; they MUST be treated as top-level rather than vanishing from channel windows. Thread mode instead requires metadata at depths 1..`depth_limit`.
- The `d` tag on `39006` differs per request cursor by design: concurrent pages of one channel coexist in a replaceable-event cache instead of clobbering each other. The per-channel-singleton alternative would make page N overwrite page N+1's bounds.

## Relation to Other NIPs

- **NIP-01**: Supplies the filter grammar this NIP extends and the parameterized-replaceable semantics overlays lean on. (Degradation safety comes from this NIP's explicit downgrade-and-retry rule, not from assuming universal unknown-field tolerance.)
- **NIP-29**: Supplies the channel model (`h` tags, group-scoped reads) windows are scoped by.
- **NIP-50** and relay-side search: sibling precedent — a relay-computed view requested through extended filter fields, invisible to relays that do not implement it.
- **NIP-98**: Authenticates the HTTP query surface Buzz serves windows on.
- **NIP-11**: Names the relay identity that signs overlays and the natural place to advertise support.

## Recovering summaries after deletion

The HTTP bridge additionally supports `resolve_thread_roots: true` with
`kinds: [39005]`, exactly one accessible `#h` channel, and 1–100 full event
`ids`. The IDs identify target replies, including soft-deleted replies,
not the IDs of the returned summary events. The request permits at most
100 targets across all such filters.

The relay resolves ownership from retained thread metadata on the writer,
checks both target and root channel scope, and returns one signed
`kind:39005` summary per distinct owning root, including zero counts.
It returns no original target content, author, or signature. Missing or
non-reply targets and inaccessible channels produce no summaries.
These reads use the writer so deletion recovery does not depend on
replica replay or the delivery of a live summary. No bounds event is returned.

Clients use this bounded metadata operation when a deletion target has
left their reply cache. Ordinary event queries exclude tombstones and
cannot perform this recovery. This extension requires a supporting relay;
clients must not interpret an unsupported/empty response as proof that an
unknown target's thread is empty.
