# Bridge `/query` Extension: Channel Window

> **Normative spec:** [NIP-CW](nips/NIP-CW.md) is the canonical, standalone
> specification of the channel window (kinds 39005/39006, filter extension,
> cursor and trust semantics). This document remains as the ratified
> engineering contract and internal design record; where wording differs,
> NIP-CW governs.

Status: frozen contract v2 (2026-07-03) — GUI read-model overhaul.
Reviewed by: Mari (relay ground truth), Wren (client core), Quinn (spec
guardian), Perci (NIP landscape). Ratified in
`#buzz-gui-formal-relay-interaction-spec`, thread `a7c68013`.

The channel window is how Buzz clients page a channel timeline by
**top-level rows** instead of raw events. It is a raw-filter extension on
the existing HTTP bridge `POST /query` — the same extension family as
`before_id` and `thread_cursor`. There is no new endpoint, and the wire
carries only signed nostr events.

Vanilla NIP-01 cannot express "messages with no reply e-tag" (filters have
no negation), which is why generic nostr clients page raw events and
reassemble threads client-side. This relay computes `thread_metadata`
(depth, root, reply counts) at ingest, so it can serve the top-level view
directly. The WS REQ path ignores all fields below via `nostr::Filter`'s
unknown-field behavior — generic clients degrade gracefully to a normal
full-event query; they never see a wrong-but-plausible timeline.

## Request

A standard bridge filter plus extension fields:

```json
{
  "kinds": [9],
  "#h": ["<channel-uuid>"],
  "limit": 50,
  "top_level": true,
  "include_summaries": true,
  "include_aux": true,
  "until": 1751500000,
  "before_id": "<64-hex event id>"
}
```

- `top_level: true` — routes this filter to the top-level SQL view.
  Requires exactly one `#h` channel the caller can access.
- `limit` — row budget. Counts **row events only**; summaries, aux, and
  bounds overlays never consume it.
- `until` + `before_id` — the composite request cursor `(created_at, id)`
  of the last retained row from the previous page. **Both or neither.**
  `top_level` with `until` but no `before_id` is rejected (`400`): the
  window path has no timestamp-only fallback, ever. Neither = head request.
- `direction` — `"older"` (default) pages newest first; `"newer"` pages
  oldest first, from the channel's first row. A relay that supports
  `"newer"` advertises `buzz-window-newer` in NIP-11
  `supported_extensions`; clients send it only then.
- `after` + `after_id` — the `"newer"` cursor, with the same rules as
  `until` + `before_id`: both or neither, no timestamp-only fallback. A
  `"newer"` request with `until`/`before_id`, or an `"older"` one with
  `after`/`after_id`, is rejected (`400`).
- `include_summaries` / `include_aux` — opt-in overlay/closure appends.

`page`/OFFSET is not honored on the window path.

## Top-level predicate

A row is top-level iff `depth IS NULL OR depth = 0 OR (depth = 1 AND
broadcast = true)` in `thread_metadata` (v1 ruling: `NULL` — an event
ingested before thread metadata existed — counts as top-level; the harness
`legacyReply` scenario decides whether a backfill migration is needed).
Deleted rows (`deleted_at IS NOT NULL`) are excluded before the limit.

## Ordering and cursor

Rows are ordered `(created_at DESC, id ASC)` — the same composite every
other read path uses. The next-page cursor is the `(created_at, id)` of the
**last retained row**; the server echoes it in the `39006` bounds overlay
as `next_cursor`. Keyset comparison is
`created_at < $ts OR (created_at = $ts AND id > $id)`; dense seconds
paginate without loss or duplication by construction.

A `"newer"` window is the exact mirror: rows are ordered `(created_at ASC,
id DESC)`, and the keyset is
`created_at > $after OR (created_at = $after AND id < $after_id)`. Its
`next_cursor` is the last retained row, sent back as `after` + `after_id`.
Reading the same channel from both ends meets without a gap or overlap
while no rows are being written. A row accepted with a past `created_at`
after a page was served can sort behind that page's cursor; see NIP-CW
§Direction.

`has_more` is a **server fact**: the relay probes `limit + 1` rows after
all predicates (access, deletion, top-level, kinds), returns at most
`limit`, and reports the probe result in `39006`. The sentinel row never
reaches the wire, and no closure is computed for it. Clients must not
infer exhaustion from row count (`rows < limit` does not imply anything on
an exact-multiple final page) — `39006.has_more` is the only authority.

## Response

The existing flat bridge shape: a JSON array of signed nostr events.
Clients **partition by kind before any cursor math**:

1. **Rows** — the top-level events, in keyset order.
2. **Aux closure** (`include_aux`) — reactions (7), deletions (5, 9005),
   and edits (40003) targeting the retained rows by `#e`, **plus**
   deletions targeting those aux events (the transitive second hop, e.g.
   a delete-of-a-reaction). One round trip; no client `#e` fan-out. Each
   hop is drained server-side across the DB page clamp, so the closure is
   complete rather than newest-1000.
3. **Thread summaries** (`include_summaries`) — one relay-signed
   `kind:39005` per row that has replies.
4. **Window bounds** — exactly one relay-signed `kind:39006` per window
   response.

### `kind:39005` — thread summary overlay

- tags: `["e", <root-id>]`, `["d", <root-id>]`, `["h", <channel-id>]`
- content: `{"reply_count":n,"descendant_count":n,"last_reply_at":ts|null,"participants":["<hex-pubkey>",...]}`
  (participants: up to 10 from the newest 1000 replies, deleted ones included, most recent first)
- Signed by the relay keypair. Synthesized at query time, **never stored**.
  Clients treat it as replace-by-target metadata keyed by the `e`/`d` tag
  (the `d` tag gives parameterized-replaceable semantics natively); it is
  never a row, never a cursor input, never durable timeline history.

### `kind:39006` — window bounds overlay

- tags: `["d", "<channel_id>:<request-cursor-or-head>"]`, `["h", "<channel_id>"]`
- content: `{"has_more": bool, "next_cursor": {"created_at": ts, "id": "<hex>"} | null}`
- `next_cursor = null ⇔ has_more = false`.
- `d`-tag suffix serialization (canonical): `head` for a head request,
  else `<created_at>:<event_id>` — decimal unix seconds, then the full
  64-char lowercase hex id, colon-delimited. Clients must verify the
  suffix equals the cursor they sent and reject the overlay on mismatch.
- A `"newer"` page puts `newer:` before the suffix
  (`<channel_id>:newer:head`, `<channel_id>:newer:<created_at>:<event_id>`)
  and adds `"direction":"newer"` to the content. An `"older"` page carries
  neither, so its overlay is byte-identical to one from a relay without
  `"newer"`.
- Reserved field: `oldest_retained` (retention gap), added without a wire
  break if needed.
- Same overlay rules as 39005: relay-signed, query-time, never stored,
  never a row or cursor input.

Both kinds are relay-only: client submission is rejected at ingest.

## Client obligations (frozen)

- Pages are immutable authoritative history chained cursor→cursor; live
  events land in a separate overlay, never spliced into pages.
- Reconnect refetches page 0 and re-arms the live subscription
  (`since: now`); deeper pages need no repair path. An oldest-first
  (`direction: "newer"`) reader follows NIP-CW's direction rules instead:
  a repair read of the crossed range once the scan reaches the newest rows;
  on reconnect it reads the newest rows it missed newest first.
- Replies never enter the channel timeline; the thread panel uses the
  existing `thread_cursor` surface (#1418). Thread filters may opt into
  `include_aux` to append the same authorized two-hop reactions, edits, and
  deletions closure as a channel-window response.

## Siblings

`before_id` (requires `until`), `thread_cursor`/`thread_cursor_id`,
`depth_limit`, `feed_types` — see `bridge.rs`. All are bridge-only raw
filter extensions invisible to vanilla relays and clients.
