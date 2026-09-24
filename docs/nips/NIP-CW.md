NIP-CW
======

Channel and Thread Windows
--------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format, filters), NIP-11 (relay information document), NIP-29 (relay-based groups), NIP-98 (HTTP auth)

## Abstract

This NIP defines two relay-computed, cursor-paged views served as ordinary signed Nostr events through extended NIP-01 filters:

- **channel mode** pages a channel's timeline. By default, it returns top-level rows; a relay MAY apply a community-level policy that projects thread replies into the same channel window as flat chronological rows.
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

The channel-mode thread-reply projection policy, when enabled, is a read/display projection over stored reply events. It is not a duplicate ingest or fan-out mechanism.

This NIP does not require WebSocket REQ support. A relay MAY serve window filters only on an HTTP query surface and ignore the extension fields on REQ (see §Degradation).

## Terminology

This document uses MUST, MUST NOT, SHOULD, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The keypair whose pubkey the relay advertises (e.g. NIP-11 `self`). All overlay events are signed with it.
- **row**: A stored, signed event returned as part of the page proper (usually client-authored; Buzz also stores relay-signed events carrying actor provenance). Rows are the only events that count against `limit`.
- **top-level**: An event that opens a thread rather than replying into one — defined by wire tags in §Channel-mode Top-level Classification.
- **projected thread reply**: A reply event that remains stored only as a thread reply, but is also returned as a flat row in the channel window because the channel's community has enabled thread-reply projection.
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

### Community Row Mode

The request does not choose whether thread replies are projected into the channel window. That choice is a community-level relay policy. Buzz exposes it as the community profile field `thread_replies_in_channel`, which may be changed only by community owners/admins and defaults to `false` for existing communities.

A served window response MUST echo the policy used for that page in the `kind:39006` bounds content as boolean `thread_replies_in_channel`. The echoed value is part of the window mode: a cursor obtained while it is `false` MUST NOT be reused to continue a window after the value becomes `true`, or vice versa. Clients that observe a mode change while appending older pages MUST discard or refetch the affected window chain rather than mixing pages fetched under different row predicates.

## Channel-mode Top-level Classification

The row set must be reproducible from wire data alone, so the reply/top-level distinction is defined by tags, not by any relay's storage schema.

An event is a **reply** iff it carries a NIP-10 *marked* `e` tag with the `reply` marker (`["e", "<parent-id>", <relay-url>, "reply"]`, parent id being 64 hex characters). An event with no marked `reply` e-tag — including one carrying only a `root`-marked tag, unmarked/positional e-tags, or no e-tags at all — is **not** a reply.

From that predicate:

- **depth** 0 = not a reply. A reply's depth is its parent's depth + 1, following `reply` markers up the ancestry (relays MAY cap depth; Buzz rejects beyond 100). A reply MUST target a parent in the same channel; its `root` marker, when present, MUST agree with the parent's ancestry.
- **broadcast**: a reply is *broadcast to the channel* iff it carries the exact tag `["broadcast", "1"]`. Broadcasting is an author's opt-in to surface a depth-1 reply on the channel timeline as well as in its thread.

In the default community row mode, an event is **top-level** — eligible to be a window row — iff its depth is 0, or its depth is 1 and it is broadcast.

When `thread_replies_in_channel` is enabled for the community, all non-deleted events in the target channel that match the request's row kind filter are eligible window rows, including direct and nested thread replies. A projected thread reply MUST appear at its own position in the total channel-window order, as a peer row, not nested or grouped under its root. The `["broadcast", "1"]` tag does not create a second row in projection mode; it is only the default-mode opt-in that admits a depth-1 reply to the top-level row set.

Storage fallback (fail-open): a relay that indexes this classification at ingest may hold events stored before the index existed, whose depth is unknown. Such events MUST be treated as top-level rather than vanishing from every window. This is a compatibility rule for pre-index data, not a third protocol state — an interoperating implementation classifying from tags alone has no unknown case.

## Channel-mode Relay Processing Algorithm

For a valid window filter on an accessible channel (§Channel-mode Access Scoping) the relay MUST:

1. **Select rows.** Determine the community row mode before applying the row budget. If `thread_replies_in_channel` is false, take events that are top-level (§Channel-mode Top-level Classification), not deleted, and matching `kinds` if present. If it is true, take all non-deleted events in the target channel that match `kinds` if present, including replies at any depth. In both modes, order rows by `created_at DESC, id ASC` (`id` compared bytewise). With a cursor `(ts, id)`, retain only events where `created_at < ts OR (created_at = ts AND id > id)`.
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
  "content": "{\"has_more\":true,\"next_cursor\":{\"created_at\":1751499000,\"id\":\"<64-hex id>\"},\"thread_replies_in_channel\":false}"
}
```

- `d`-tag suffix (canonical serialization): the literal string `head` for a head request, else `<created_at>:<event_id>` — decimal unix seconds, colon, full 64-character lowercase hex id — identifying the *request* cursor this page answered. Clients MUST verify the suffix equals the cursor they sent and discard the overlay (and the page) on mismatch; this binds each bounds overlay to its request and makes concurrent-page responses unambiguous.
- `next_cursor` — the composite cursor to echo as `until` + `before_id` for the next page, or `null` iff `has_more` is `false`.
- `thread_replies_in_channel` — boolean community row mode used to compute the page. Missing or `false` means the default top-level row predicate. `true` means thread replies were eligible rows before limit/probe/cursor evaluation. Clients MUST treat a change in this value as a window-mode change and refetch rather than mixing pages from both modes.
- Reserved: an `oldest_retained` content field may be added (retention gap signaling) without a wire break. Clients MUST ignore unknown content fields.

## Channel-mode Client Behavior

1. **Head request**: send the window filter with no cursor. Render rows in received order.
2. **Continue**: read `kind:39006`; if `has_more`, send the same filter with `until = next_cursor.created_at`, `before_id = next_cursor.id`. Repeat until `has_more = false`.
3. **Exhaustion**: `39006.has_more` is the only exhaustion signal. `rows < limit` proves nothing — an exact-multiple final page returns `limit` rows with `has_more = false`, and predicate filtering can shrink any page. A client MUST NOT stop paging on row count, and MUST NOT treat a full page as "more available."
4. **Immutability**: fetched pages are immutable history chained cursor→cursor. New live events MUST NOT be spliced into fetched pages; deliver them through a separate live subscription (`since: now`) and merge at render time. On reconnect, refetch the head page and re-arm the live subscription; deeper pages need no repair.
5. **Bounds integrity**: a window response missing its `kind:39006`, or carrying more than one, or carrying one whose `d`-tag binding does not echo the request cursor, whose content is not parseable JSON, or whose content violates `has_more = true ⇔ next_cursor ≠ null`, is not a usable page — the client MUST discard it (and MAY retry) rather than guess at exhaustion. Clients SHOULD additionally reject overlays that violate the exact tag cardinality of §Channel-mode Overlay Event Formats or whose content fields have the wrong runtime types (hardening against a malformed or hostile serializer). Cryptographic verification is governed by §Overlay Trust.
6. **Overlays are metadata**: never render a `39005`/`39006` as a message, never feed one into cursor math, and key cached summaries by their `d` tag (latest wins).
7. **Projection presentation**: when `thread_replies_in_channel` is true, a reply row SHOULD be rendered as a normal chronological channel row with lightweight context linking it back to the root/original thread. Reply actions from such a row SHOULD target the original thread/reply. Opening the side thread view remains a client affordance, but clients MUST NOT publish duplicate channel events merely to represent the projection.

## Legacy Oldest-first Threads

Buzz's authenticated `POST /query` also supports an older thread path, separate
from both window modes. It is selected by a single `#e` root and `depth_limit`,
with `thread_window` absent or false:

```jsonc
{
  "#h": ["<channel UUID>"],
  "#e": ["<root event id>"],
  "kinds": [9, 40002],
  "depth_limit": 100,
  "limit": 100,
  "include_aux": true,
  "thread_cursor": 1751500000,
  "thread_cursor_id": "<last loaded reply id>"
}
```

- Omit both cursor fields to start at the oldest reply. The historical
  `thread_cursor: -1` start sentinel remains accepted. `depth_limit` is an
  explicit maximum depth; `2147483647` is the existing unbounded-depth sentinel.
  The row limit defaults to 100 and is capped at 500.
- Replies are ordered by `created_at ASC, id ASC`. To continue, derive
  `thread_cursor` and `thread_cursor_id` from the **last loaded reply**, not
  from an auxiliary event. The next page satisfies `created_at > cursor OR
  (created_at = cursor AND id > cursor_id)`. The camel-case aliases
  `threadCursor` and `threadCursorId` are also accepted.
- Timestamp-only continuation remains accepted for compatibility but skips
  other replies in the same second; clients SHOULD send the composite pair.
- This path traverses stored thread metadata, not the newest-first mode's strict
  row-kind filter. Clients SHOULD supply explicit `kinds` for query authorization
  but MUST NOT assume this legacy thread path restricts its reply rows by them.
- `include_aux` appends root/reply reactions, edits and deletions, followed by
  deletions of auxiliary events. These do not count against the reply limit.
- There is **no signed bounds event or server-issued continuation cursor**.
  A full reply page can be the final page, so continuation may require one more
  request. A short/empty reply page is the legacy stop heuristic, not a signed
  exhaustion fact: access filtering can also shorten a response.

A client explicitly falling back from thread windows MUST restart this path
from the oldest reply with clean pagination state. `until`/`before_id` are not
legacy thread continuation fields and MUST NOT be reused as such.

## Thread Mode

`thread_window: true` requests a newest-first page of replies to one root. It is
served through Buzz's NIP-98-authenticated `POST /query`; channel mode,
`kind:39006`, and the legacy oldest-first thread path are unchanged. The response
is a flat array of reply events, optional auxiliary events, and exactly one
relay-signed `kind:39007` bounds event. This specification adds no endpoint,
subscription, or around-target query.

### Request

```jsonc
{
  "thread_window": true,
  "#h": ["<channel UUID>"],
  "#e": ["<root event id>"],
  "kinds": [9, 40002],
  "depth_limit": 100,
  "limit": 50,
  "include_aux": true,
  "until": 1751500000,
  "before_id": "<64-hex id>"
}
```

`#h` and `#e` MUST each contain exactly one value. `kinds` MUST contain one to
four entries from `9`, `40002`, `45001`, and `45003`; relays normalize them to
a sorted, distinct list. `limit` defaults to 50 and MUST be 1–200;
`depth_limit` defaults to 100 and MUST be 1–100; `include_aux` defaults to
false. A continuation MUST provide both `until` and `before_id`, or neither for
the head page. Unknown fields, malformed values, and mixing thread windows with
another query mode MUST be rejected. A query MAY contain at most four window
filters.

### Relay Processing

For an authorized request the relay MUST:

1. Verify that the root is a supported conversation event (`9`, `40002`,
   `45001`, or `45003`) in the requested community and channel. Retained root
   tombstones are eligible. A missing, out-of-scope, or unsupported root
   (including a `40008` diff) returns no events or bounds; it MUST NOT receive
   a signed exhausted page even if ingest has threaded replies beneath it.
2. Select non-deleted replies in that channel at depths 1 through
   `depth_limit`, restricted by `kinds`, ordered by `created_at DESC, id ASC`.
   A continuation retains rows where `created_at < until OR (created_at = until
   AND id > before_id)`.
3. Evaluate `limit + 1` after all predicates. Discard the extra row and set
   `has_more`; when more rows exist, `next_cursor` is the last retained scan
   candidate, captured before event reconstruction. Otherwise it is `null`.
4. If `include_aux` is true, append reactions (`kind:7`), deletions
   (`kind:5`/`9005`), and edits (`kind:40003`) targeting the root or returned
   replies, followed by deletions targeting those auxiliary events. Preserve
   original signatures, deduplicate by event ID, and apply access control to
   every event. A target reference requires an `e` tag with the target ID
   in its second position; containing both strings elsewhere is insufficient.
5. Append one bounds event to each served page, including empty and exhausted
   pages. Refresh access for the **entire query batch** before releasing any
   events or bounds. If the reader's accessible-channel set changed while any
   window was built, discard all accumulated output and return a retryable
   error (Buzz: HTTP `503`). This includes cross-channel auxiliary access.

Rows alone count against `limit`. Auxiliary events and bounds do not.
Inaccessible and nonexistent channels return no events or bounds.

### Resource Limits and Recovery

Buzz shares resource limits across all thread-window filters in one query,
including replica retries: 64 auxiliary SQL scans, 8,192 raw reply and auxiliary
rows (including probes), 8 MiB of their combined content plus serialized tags, 8 MiB
of serialized output, and an eight-second overall deadline. Reply and auxiliary
payloads are consumed incrementally and charged before reconstruction; tombstones and
probes consume the raw allowance too. No partial page or bounds is returned
when a budget is exceeded.

Resource exhaustion and database timeouts return HTTP `503`; malformed stored
auxiliary data remains a separate HTTP `500`. Clients MUST discard the entire
failed batch without advancing any cursor. For work/byte exhaustion, reduce
`limit`, split the batch, or explicitly request `include_aux: false` and fetch
needed auxiliary data separately. A root's auxiliary history can exceed the
allowance even at `limit: 1`; blind retries of the same query will not help.
For timeouts or changed access, retry with bounded backoff and fresh access.
These errors MUST NOT trigger legacy compatibility fallback.

### Thread Bounds: `kind:39007`

Bounds are synthesized per query and MUST NOT be accepted at ingest. Tags are
exactly one `d`, one `h`, and one `e`:

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

The binding is SHA-256 over compact UTF-8 JSON of this ordered array:

```jsonc
["tw",1,"older","<host>","<reader hex>","<channel>","<root>",50,100,[9,40002],null,true]
// host, reader, channel, root, limit, depth, sorted kinds, request cursor, include_aux
// cursor is null or [<seconds>,"<lowercase id>"]
```

`host` is the server-resolved normalized authority and `reader` is the
authenticated lowercase pubkey. Clients MUST verify the expected relay signer
and signature, exact tags and request binding, version, direction, and
`next_cursor == null` iff `has_more == false`. Only validated bounds determine
exhaustion; clients MUST echo `next_cursor` as `until` and `before_id`.

An extension-unaware relay may return legacy oldest-first history without
bounds. A client that explicitly falls back MUST restart with clean legacy
pagination state and MUST NOT reuse a descending cursor. Invalid signatures,
authorization failures, timeouts, corruption, or incomplete auxiliary closure
MUST NOT trigger compatibility fallback. Buzz currently ships no thread-mode
client opt-in or fallback implementation.

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

- The `limit + 1` probe MUST run after *all* predicates: access, deletion, and `kinds` in both modes; row mode and top-level classification in channel mode; root, channel and depth restrictions in thread mode. A probe over a superset produces false `has_more = true` on the last page.
- The community `thread_replies_in_channel` policy is part of the row predicate and MUST be applied before `limit + 1`, cursor derivation, aux closure, and summaries. Mixing pages fetched under different values can skip or duplicate rows.
- The cursor comparison uses `id > $id` (bytewise ascending) because the total order is `created_at DESC, id ASC`. Getting the id inequality backwards drops or duplicates same-second rows — precisely the bug the composite cursor removes.
- `next_cursor` is the last retained *scan candidate*, not the last delivered row: capture the scan position before per-event reconstruction so a skipped event cannot stall pagination. Clients echo it verbatim and never derive or validate it against the rows they received.
- **Channel mode only:** events ingested before the relay computed thread metadata have no depth; they MUST be treated as top-level rather than vanishing from channel windows. Thread mode instead requires metadata at depths 1..`depth_limit`.
- The `d` tag on `39006` differs per request cursor by design: concurrent pages of one channel coexist in a replaceable-event cache instead of clobbering each other. The per-channel-singleton alternative would make page N overwrite page N+1's bounds.
- Broadcast and projection are distinct. In default mode, broadcast admits only a depth-1 reply to the channel row set. In projection mode, all replies are already eligible rows and a broadcast tag MUST NOT produce an additional duplicate row.

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
