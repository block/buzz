NIP-CW
======

Channel Window
--------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format, filters), NIP-11 (relay information document), NIP-29 (relay-based groups), NIP-98 (HTTP auth)

## Abstract

This NIP defines the **channel window**: a relay-computed, cursor-paged view of a channel's *top-level* timeline, served as ordinary signed Nostr events through an extended NIP-01 filter. One request returns a page of top-level rows in stable keyset order, optionally accompanied by the aux closure and two relay-signed overlay families:

- the **aux closure** — stored reactions, deletions, and edits targeting the returned rows, with their original authors and signatures (`include_aux`),
- **thread summaries** — one relay-signed `kind:39005` per row that has replies (`include_summaries`),
- **window bounds** — exactly one relay-signed `kind:39006` carrying the authoritative `has_more` fact and the next-page cursor.

The extension adds no endpoint and no envelope. The wire format is the flat array of signed events the query surface already returns; a client that ignores this NIP receives standard behavior everywhere.

## Motivation

A NIP-01 filter can only *match* tag values; it cannot express their absence. "Channel messages that are **not** replies" — the timeline every threaded-chat client renders first — is therefore inexpressible in vanilla filters, so generic clients page the full event stream and reassemble threads client-side. That costs bandwidth proportional to reply volume, and worse, it breaks pagination correctness: `limit` counts raw events, so a page of 50 events may contain 3 top-level rows or 50, and the client cannot ask for "the next 50 rows."

Timestamp pagination (`until` alone) has a second defect: `created_at` has one-second resolution, so bursts of same-second events make a timestamp cursor lossy or duplicative at every page boundary.

A relay that computes thread structure at ingest already knows which events are top-level. This NIP lets a client request that view directly, with a composite `(created_at, id)` cursor that is exact under same-second bursts, and with server-computed exhaustion (`has_more`) so an exact-multiple final page is not misread as "more available."

## Non-Goals

This NIP does not change ingest, storage, or fan-out. Rows returned in a window are ordinary stored events; the overlays are computed per query and never stored.

This NIP does not define thread *reading*. Replies never appear as window rows; fetching a thread's contents is out of scope.

This NIP does not require WebSocket REQ support. A relay MAY serve window filters only on an HTTP query surface and ignore the extension fields on REQ (see §Degradation).

## Terminology

This document uses MUST, MUST NOT, SHOULD, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The keypair whose pubkey the relay advertises (e.g. NIP-11 `self`). All overlay events are signed with it.
- **row**: A stored, signed event returned as part of the page proper (usually client-authored; Buzz also stores relay-signed events carrying actor provenance). Rows are the only events that count against `limit`.
- **top-level**: An event that opens a thread rather than replying into one — defined by wire tags in §Top-level Classification.
- **overlay**: A relay-signed event (`kind:39005`, `kind:39006`) synthesized at query time. Overlays are metadata *about* rows: never a row, never a cursor input, never durable history.
- **composite cursor**: The pair `(created_at, id)` identifying a position in the total order. `created_at` is unix seconds; `id` is a 64-character lowercase hex event id.
- **scan position**: The composite cursor of the last event the relay's query *retained*, whether or not that event was ultimately delivered as a row (see §Relay Processing step 3). The cursor tracks where the scan stopped, not what the client received.

## Request

A window request is a standard filter plus extension fields, submitted wherever the relay accepts filters (for Buzz: the NIP-98-authenticated HTTP bridge `POST /query`):

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
- `#h` — the window MUST target exactly one channel. Zero or multiple channels: reject with an error (Buzz: HTTP `400`). A channel the requester cannot access is handled by §Access Scoping, not by an error that confirms the channel exists.
- `limit` — the row budget. Overlays and aux events MUST NOT count against it. Relays SHOULD clamp it to a documented range (Buzz: default 50, maximum 200, minimum 1).
- `until` + `before_id` — the request cursor: the `next_cursor` from the previous page's `kind:39006` overlay, echoed verbatim — `until` = `next_cursor.created_at`, `before_id` = `next_cursor.id`. **Both present or both absent.** Exactly one present MUST be rejected: a timestamp-only cursor silently loses or duplicates same-second rows, which is the failure mode this NIP exists to remove. Both absent = head-of-channel request.
- `kinds` — optional; restricts which kinds may be rows. It does not affect overlay or aux kinds.

Cursor grammar: `until` MUST be a non-negative integer of unix seconds representable by the relay's timestamp type; `before_id` MUST be exactly 64 hexadecimal characters (the lowercase form emitted in `next_cursor.id` is canonical). A malformed value MUST cause rejection of the request — it MUST NOT be ignored and demoted to a half cursor or a head request.

Offset/page-number pagination MUST NOT be honored on the window path.

## Top-level Classification

The row set must be reproducible from wire data alone, so the reply/top-level distinction is defined by tags, not by any relay's storage schema.

An event is a **reply** iff it carries a NIP-10 *marked* `e` tag with the `reply` marker (`["e", "<parent-id>", <relay-url>, "reply"]`, parent id being 64 hex characters). An event with no marked `reply` e-tag — including one carrying only a `root`-marked tag, unmarked/positional e-tags, or no e-tags at all — is **not** a reply.

From that predicate:

- **depth** 0 = not a reply. A reply's depth is its parent's depth + 1, following `reply` markers up the ancestry (relays MAY cap depth; Buzz rejects beyond 100). A reply MUST target a parent in the same channel; its `root` marker, when present, MUST agree with the parent's ancestry.
- **broadcast**: a reply is *broadcast to the channel* iff it carries the exact tag `["broadcast", "1"]`. Broadcasting is an author's opt-in to surface a depth-1 reply on the channel timeline as well as in its thread.

An event is **top-level** — eligible to be a window row — iff its depth is 0, or its depth is 1 and it is broadcast.

Storage fallback (fail-open): a relay that indexes this classification at ingest may hold events stored before the index existed, whose depth is unknown. Such events MUST be treated as top-level rather than vanishing from every window. This is a compatibility rule for pre-index data, not a third protocol state — an interoperating implementation classifying from tags alone has no unknown case.

## Relay Processing Algorithm

For a valid window filter on an accessible channel (§Access Scoping) the relay MUST:

1. **Select rows.** From the target channel, take events that are top-level (§Top-level Classification), not deleted, and matching `kinds` if present, in the total order `created_at DESC, id ASC` (`id` compared bytewise). With a cursor `(ts, id)`, retain only events where `created_at < ts OR (created_at = ts AND id > id)`.
2. **Probe exhaustion.** Evaluate the query with an internal budget of `limit + 1` rows *after all predicates*. If `limit + 1` rows match, `has_more = true` and the sentinel row is discarded — it MUST NOT appear on the wire, in overlays, or in the aux closure. Otherwise `has_more = false`.
3. **Derive the next cursor.** If `has_more`, `next_cursor` is the **scan position**: the composite cursor of the last retained candidate, captured *before* any serving-time reconstruction or filtering of individual events. Otherwise `next_cursor = null`. The invariant `next_cursor = null ⇔ has_more = false` MUST hold. Because it is a scan position, `next_cursor` MAY reference an event that does not appear in the response (e.g. one skipped by the relay as unreconstructable); it is authoritative regardless, and deriving it from delivered rows instead would stall pagination on every skipped event.
4. **Append the aux closure** (if `include_aux` and at least one row): two hops of events referencing the rows by `e` tag. Hop 1: reactions (`kind:7`), deletions (`kind:5`, `kind:9005`), and edits (Buzz `kind:40003`) whose `e` tag is a row id. Hop 2: deletions whose `e` tag is a hop-1 event id (a delete-of-a-reaction). Each event appears at most once; access-scoped events the requester cannot read are omitted. Relays MAY cap each hop (Buzz: 1000 events per hop).
5. **Append thread summaries** (if `include_summaries`): one `kind:39005` per row that has at least one reply. Rows without replies get none.
6. **Append window bounds**: exactly one `kind:39006` per served window response, always — including empty and exhausted pages.

The response is the surface's ordinary flat array of signed events — rows first in keyset order, then aux, then summaries, then bounds. Clients MUST partition by kind and MUST NOT rely on array position beyond the ordering of rows.

## Access Scoping

Access is evaluated before any of the steps above. A syntactically valid window request for a channel the requester cannot access — including a channel that does not exist — MUST produce the relay's ordinary access-scoped result for that surface, with **no rows and no overlays**. For Buzz's query surface that ordinary result is an empty array, exactly as any other filter against an inaccessible channel produces.

Two consequences implementers MUST NOT miss:

- The "exactly one `kind:39006`" guarantee applies only to *served* windows — responses where access succeeded. The absence of a bounds overlay is therefore meaningful: it tells an extension-aware client that no window was served (access-scoped, or the relay does not implement this NIP — see §Degradation).
- An inaccessible channel is thereby indistinguishable from a nonexistent one, but *not* from an accessible empty channel: the latter is a served window and does return a `39006` (`has_more: false`). This is the same existence-disclosure posture as the relay's ordinary reads — a requester who can query a channel at all was already entitled to know it exists.

## Overlay Event Formats

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

## Client Behavior

1. **Head request**: send the window filter with no cursor. Render rows in received order.
2. **Continue**: read `kind:39006`; if `has_more`, send the same filter with `until = next_cursor.created_at`, `before_id = next_cursor.id`. Repeat until `has_more = false`.
3. **Exhaustion**: `39006.has_more` is the only exhaustion signal. `rows < limit` proves nothing — an exact-multiple final page returns `limit` rows with `has_more = false`, and predicate filtering can shrink any page. A client MUST NOT stop paging on row count, and MUST NOT treat a full page as "more available."
4. **Immutability**: fetched pages are immutable history chained cursor→cursor. New live events MUST NOT be spliced into fetched pages; deliver them through a separate live subscription (`since: now`) and merge at render time. On reconnect, refetch the head page and re-arm the live subscription; deeper pages need no repair.
5. **Bounds integrity**: a window response missing its `kind:39006`, or carrying more than one, or carrying one whose `d`-tag binding does not echo the request cursor, whose content is not parseable JSON, or whose content violates `has_more = true ⇔ next_cursor ≠ null`, is not a usable page — the client MUST discard it (and MAY retry) rather than guess at exhaustion. Clients SHOULD additionally reject overlays that violate the exact tag cardinality of §Overlay Event Formats or whose content fields have the wrong runtime types (hardening against a malformed or hostile serializer). Cryptographic verification is governed by §Overlay Trust.
6. **Overlays are metadata**: never render a `39005`/`39006` as a message, never feed one into cursor math, and key cached summaries by their `d` tag (latest wins).

## Degradation

Every extension field in this NIP is an *additional* key on a standard filter, and clients and relays that do not implement it need no changes:

- **Extension-unaware relay**: a tolerant filter parser (one that ignores unknown keys, as common NIP-01 implementations do) serves the filter as a plain `kinds` + `#h` query — a complete, correct, standard event stream. A strict parser may instead reject the filter outright. Both are safe: neither produces a wrong-but-plausible top-level timeline. A client MUST treat *either* signal — a response with no valid `kind:39006`, or an error/unsupported-filter response — as a downgrade, and fall back by reissuing a clean standard filter with all extension keys removed and assembling threads client-side. (Buzz's own WebSocket REQ path is such a tolerant parser: the filter deserializer drops the extension fields, so a window filter on REQ serves the standard query.)
- **Extension-unaware client**: never sends `top_level`, never sees an overlay kind, and observes a completely standard relay.

A relay implementing this NIP MAY advertise it in its NIP-11 relay information document; the discovery mechanism is out of scope for this NIP. A client needs no advertisement to probe safely: send one head window request and apply the downgrade rule above — the presence of a valid `kind:39006` is the capability signal.

## Security and Privacy Considerations

Overlays are relay-authored facts about data the requester can already read. A relay MUST apply its normal access scoping to rows and to every aux-closure event, and §Access Scoping governs inaccessible channels: no rows, no overlays, no distinguishable error.

`kind:39005` aggregates thread activity (participant pubkeys, counts, recency) into one event. It only ever describes threads rooted in a channel the requester can read, so it reveals nothing a client could not compute from readable events — it saves round trips, not permissions.

Client-submitted `39005`/`39006` MUST be rejected at ingest (relay-only kinds); a forged overlay accepted into storage could later masquerade as relay-signed state.

### Overlay Trust

Because `kind:39006` is the pagination authority, a client MUST adopt exactly one of these trust profiles before using the window fast path:

- **Authenticated-transport profile** (what Buzz desktop ships): the client speaks to a relay it deliberately configured as its source of truth, over TLS (HTTPS/WSS) to that configured origin — server-origin authentication comes from the TLS certificate chain, which is what proves the response bytes came from the relay. (NIP-98 request signing and NIP-42 auth run over this channel too, but they authenticate the *requester* to the relay for access control; they are not evidence of response provenance.) The MUST-level structural checks of §Client Behavior step 5 — exactly one bounds, request binding, parseable content, `has_more`/`next_cursor` agreement — are still mandatory and are what #1500 enforces. The SHOULD-level checks of step 5 (exact tag cardinality, runtime field-type validation) and cryptographically binding overlay signatures to the advertised NIP-11 identity are future hardening, to be applied uniformly across all relay-signed reads (with NIP-DV, NIP-IA), not a current guarantee. Under this profile, "relay-signed" is a TLS-origin claim, not a client-verified cryptographic one.
- **Identity-verified profile**: the client has obtained and trusts the relay identity pubkey out-of-band or via NIP-11. It MUST verify each overlay's event id, Schnorr signature, and signer against that identity, and treat any failure as the §step-5 discard. This is the profile for clients that cannot or do not authenticate their transport end-to-end.

A client with neither an authenticated transport nor a verifiable relay identity MUST NOT use the window fast path: it falls back to the standard filter (§Degradation), where it verifies every event signature itself.

## Implementation Gotchas

- The `limit + 1` probe MUST run after *all* predicates (access, deletion, top-level, `kinds`). A probe over a superset produces false `has_more = true` on the last page.
- The cursor comparison uses `id > $id` (bytewise ascending) because the total order is `created_at DESC, id ASC`. Getting the id inequality backwards drops or duplicates same-second rows — precisely the bug the composite cursor removes.
- `next_cursor` is the last retained *scan candidate*, not the last delivered row: capture the scan position before per-event reconstruction so a skipped event cannot stall pagination. Clients echo it verbatim and never derive or validate it against the rows they received.
- Events ingested before the relay computed thread metadata have no depth; they MUST be treated as top-level rather than vanishing from every window.
- The `d` tag on `39006` differs per request cursor by design: concurrent pages of one channel coexist in a replaceable-event cache instead of clobbering each other. The per-channel-singleton alternative would make page N overwrite page N+1's bounds.

## Relation to Other NIPs

- **NIP-01**: Supplies the filter grammar this NIP extends and the parameterized-replaceable semantics overlays lean on. (Degradation safety comes from this NIP's explicit downgrade-and-retry rule, not from assuming universal unknown-field tolerance.)
- **NIP-29**: Supplies the channel model (`h` tags, group-scoped reads) windows are scoped by.
- **NIP-50** and relay-side search: sibling precedent — a relay-computed view requested through extended filter fields, invisible to relays that do not implement it.
- **NIP-98**: Authenticates the HTTP query surface Buzz serves windows on.
- **NIP-11**: Names the relay identity that signs overlays and the natural place to advertise support.
