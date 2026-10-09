# Private read-state accessory API

`BUZZ_V1_ENABLED=true` opts into `/buzz/v1`; it is disabled by default.
Conversation history, live events, edits and deletion remain Nostr-authoritative.
This API neither replaces Nostr reads nor writes artificial signed events.
Legacy NIP-RS continues unchanged, but does not synchronize with these tables.

## Discovery and identity

On a known community host, NIP-11 (`GET /` with `Accept:
application/nostr+json`, or `GET /info`) includes `buzz_v1` only when enabled:

```json
{"buzz_v1":{"version":1,"base_path":"/buzz/v1","retention_seconds":2592000,
"max_channels":20,"max_intents":100,"eligible_kinds":[9,40002,45001,45003]}}
```

Use the requesting origin plus this relative prefix. Discovery is a configured
capability, not a promise that the next request cannot fail. Unknown hosts and
disabled deployments omit it, and a disabled deployment does not mount
`/buzz/v1` at all. Absence means read state is unavailable, not that everything
is read: show no count rather than zero, and keep unsent intents. A client may
also speak NIP-RS, which the relay still serves, but the two never synchronize;
buzz-app uses v1 only.

Every API request requires NIP-98, including on development relays. Sign the
exact externally addressed URL, including the encoded query, and method. POST
also requires the SHA-256 payload tag for the exact body bytes. Each retry needs
fresh authorization; replay protection is shared with the bridge. Applicable
NIP-FI admission is enforced and its asserted key must match the request signer.
The host chooses the community; the signer chooses `/me`. NIP-OA admission does
not grant access to the owner's personal state. Relay membership, bans and
resource access are enforced; moderation timeouts do not prohibit reading.

Responses produced by the v1 handlers are `Cache-Control: private, no-store`.
Application errors (including NIP-98 failures with NIP-FI Off or Shadow) use
`{"error":{"code":"invalid_request","request_id":"..."}}`, with 400 invalid,
401 unauthorized/replay, 403 forbidden, 404 unavailable capability/host/path,
429 rate limited or 503 temporarily unavailable. Application 429/503 errors
include `Retry-After`. When NIP-FI restricts (Enforce or DenyProtected),
admission failures instead preserve
the shared [NIP-FI HTTP denial contract](nips/NIP-FI.md): status, fixed plaintext
body, `Content-Type` and (for 401) `WWW-Authenticate: Nostr`. The v1 handler adds
`private, no-store` without changing those fields, unlike the bridge's direct
passthrough. Denials from the outer shared router middleware use its common
response policy, not the v1 handler's cache or JSON policy.
Unknown request fields are rejected. Never turn a transport failure into read.

## Sidebar

`GET /buzz/v1/me/sidebar?limit=20&cursor=<exclusive-channel-uuid>` returns
`channels` and `next_cursor`. Omit the cursor on the first request. Only
joined, nondeleted channels are listed; names, types, archived and hidden state
come from Nostr channel metadata, not this API. Each page has a
writer-consistent snapshot; separate pages do not share a snapshot, and an
unfinished traversal cannot prove channel removal.

`GET /buzz/v1/me/sidebar?channel_ids=<uuid>,<uuid>` refreshes 1–20 unique
channels in one snapshot, ordered by ID with `next_cursor: null`. It cannot be
combined with `limit` or `cursor`. A requested ID absent from the result was not
a joined, nondeleted, accessible sidebar row at that snapshot: remove its row.
Absence says nothing else about access to an open channel.

```json
{"channels":[{"channel_id":"<uuid>","unread":true,"mentions":1,
  "read_through_id":"<64-hex>","latest_id":"<64-hex>",
  "threads":[{"root_id":"<64-hex>","unread":true,"mentions":2,
    "read_through_id":null,"latest_id":"<64-hex>"}]}],
 "next_cursor":null}
```

A channel's timeline and each of its threads keep independent read positions;
marking one never moves another.

| Field | Channel row | Thread row |
|---|---|---|
| `unread` | an unread top-level message counts | an unread reply counts |
| `mentions` | unread top-level messages directed at you | unread replies that count |
| `read_through_id` | the anchor of the timeline's frontier, or null | the anchor of the thread's frontier, or null |
| `latest_id` | the last eligible top-level message to arrive, or null | the last counted unread reply to arrive |

A thread reply never makes the channel row unread and never becomes its
`latest_id`, including a reply carrying `broadcast=1`: it shows on its thread
row. `latest_id` is the anchor that reads its scope: "mark channel read" sends a
channel `mark_through` at the channel's `latest_id`, plus one thread
`mark_through` at each listed thread's `latest_id` to clear those too. The
channel `latest_id` counts any author and ignores read progress. Null means the
bounded scans found no top-level message, not that the channel is empty.
`read_through_id` may name a message that has since been deleted.

`threads` lists threads with unread replies that count, newest unread reply
first by author time (then `root_id`), at most 25. Threads past the 25th are
omitted with their mentions, and nothing signals the omission: after marking
the listed threads read, a refresh can list the next ones. A thread row's
`unread` is always true in this version. No message bytes are included.

A message is eligible when it is non-own, nondeleted, of the advertised
`eligible_kinds` and inside the horizon. The same kinds alone define latest
activity, so an edit, reaction or diff (40008) neither makes a channel unread
nor moves it. Classify live arrivals with the advertised set, not a client copy.

A top-level message is directed at you when its channel is a DM, it tags you
with `p`, or it carries `broadcast=1`. A reply counts only when it is directed
in the same way, or it is in one of your conversations: you wrote its direct
parent or have a reply to that same parent, in that channel. Conversation
membership uses only your live eligible messages (a deleted parent proves
nothing; a surviving reply to it still does), looks at the direct parent only
(owning the root or replying elsewhere in the thread proves nothing), and is
independent of read progress and retention. This is not Desktop notification
policy: follows and mutes do not affect these counts.

Counts are what the bounded scans could establish, and may undercount. A message
whose tags are unusable, whose ancestry is unresolved, or whose conversation
membership is undecided is left out, as are messages beyond the scan bounds
(see [Bounds](#bounds-and-deployment)).

The unread horizon defaults to 30 days (`BUZZ_V1_RETENTION_SECONDS`) and is
measured in author time (`created_at`). It filters unread and mentions, not
latest activity, event storage or frontier state. Frontiers use a different
clock, relay arrival (see [Order](#order)). Three consequences:

- A message accepted late with an author time beyond the horizon (an import, a
  backfill, a long-offline sender) is excluded under the current horizon,
  however recently the relay accepted it. It can still be latest when the
  horizon holds no message.
- Unread expires at author time plus the horizon, so a future-dated author
  time extends how long a message counts.
- A context's unread set is two tests, not one range: arrived after the
  frontier, and author time at or after the cutoff.

A later configuration expansion can change counts without having lost progress.

## Fixed-operand writes

`POST /buzz/v1/me/read-state` accepts 1–100 independent intents:

```json
{"intents":[
 {"type":"mark_through","target":{"channel_id":"<uuid>"},"message_id":"<64-hex-event>"},
 {"type":"mark_through","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"},"message_id":"<64-hex-event>"}
]}
```

The response is `{"outcomes":[...],"channels":[...]}`. Each intent commits
atomically and returns its own `applied`, `blocked` or `invalid` outcome, in
request order. An ambiguous timeout/storage failure returns
`{"status":"unknown","retryable":true}`. Earlier committed outcomes survive
later failures. `channels` holds the updated sidebar row of each distinct
channel with an applied intent, in the sidebar's row shape and ordered by ID;
a channel you have not joined has no row. Replace those rows rather than
re-deriving them. `channels` is omitted when the rows could not be read within
the request's deadline after the writes committed: refresh them with `?channel_ids=`. Retry the same
operands, never substitute latest. Keep pending intent durably on the client
until its outcome is resolved.

A mark-through validates a fixed message and advances its context's monotone
frontier to that message's relay arrival (see [Order](#order)). A channel
anchor must be a top-level message; a thread anchor must be the thread's root
or one of its replies. The frontier records the anchor as `read_through_id`
when it advances; an anchor that arrived at or before the current frontier
changes nothing. Opening a view is not itself a reading action; client
dwell/focus policy determines when to send an actual observed anchor. Old or
deleted valid anchors may advance a frontier.

### Order

A frontier is the relay arrival time (`events.received_at`) of the message a
context was read through, never its author time, which the sender chooses and
the relay accepts up to 15 minutes either way. Everything that arrived at or
before the anchor is read, whatever its author time. A message that arrives
later is unread even when backdated, and a future-dated anchor reads nothing
that arrives after it.

The order is the relay's and is not exposed: a response carries the anchor
message (`read_through_id`), never its arrival time, and no field lets a client
compute what a mark will cover. Send the anchors the user actually saw and let
the relay take the greatest. Do not compare author times, IDs or local receipt
order to drop one pending anchor in favor of another.

Arrival is the accepting relay process's clock, read just before the insert, at
microsecond resolution. Three limits follow, none of which strands a badge:

- It is not commit order. An insert that commits after a later-stamped message
  was already marked read lands read.
- Relay processes with different clocks can stamp out of true order by their
  skew.
- Messages with the identical stamp are read together.

A channel's `latest_id` is the last top-level message to arrive among those the
unread count examined: the 4,096 most recent events by author time inside the
horizon. So marking through it reads every top-level message counted. When
that scan holds no top-level message, it is the last top-level message to
arrive among the channel's 256 most recent events.

There is no import of earlier client read state. The relay records the
account's `started_at` at its first applied read intent and never moves it.
Every frontier is floored at `started_at`: until the account starts nothing
counts as unread, and from then it starts caught up, so a context it has never
marked counts only arrivals after the start. Manual unread remains
device-local.

## Bounds and deployment

- 20 sidebar rows and 100 intents per request; 25 thread rows per channel, so
  a POST's rows for 100 channels fit the response limit.
- 64 KiB write body; 1 MiB serialized API response.
- 4096 raw events per channel inside the horizon, before eligibility.
- The latest fallback probes 256 events; a long ineligible or reply-only tail
  can leave `latest_id` null.
- Tag documents over 8192 bytes or malformed relevant tags leave the message
  out. Compact boolean facts cross the database boundary, never raw tag
  payloads.
- Conversation membership: at most 1024 unique parents per request, with a
  500 ms savepoint budget. The lookup is exact, so its work grows with the
  replies under each parent. Past either bound a reply is undecided and left
  out.
- DB statement/lock deadlines and HTTP read deadlines bound work; a write's intents
  and its row refresh share one eight-second deadline after admission. Limits are
  containment, not a production capacity claim.

Apply migrations 0056 through 0058 (or the equivalent desired schema). 0056
creates two empty private tables and no index on `events`, 0057 adds the
nullable `personal_read_accounts.started_at` column, and 0058 adds the nullable
`personal_read_frontiers.through_message_id` column and drops
`threads_through_timestamp`. Both sidebar scans are
served by the existing `idx_events_community_channel_created`. No new
per-message ingest write path or stored unread counters are introduced.

Use existing HTTP route/status/latency metrics for `/buzz/v1/me/sidebar` and
`/buzz/v1/me/read-state`, plus database pool/statement metrics. No payload,
actor, channel or frontier values should become metric labels. The measured
local seed is not a DAU/concurrency or p95/p99 production acceptance result.

## Compatibility and extension rules

Within `/buzz/v1`, clients must ignore unknown response object fields. The
shape is stable: existing fields, their types, status variants and what a
`mark_through` does remain. What the counts include is not yet: which messages
are unread or directed, which threads are listed, and the bounds may change
while v1 is pre-release. A client therefore replaces its rows with each
response rather than reconciling them with its own counts. Breaking shape
changes require an explicitly negotiated contract or a new API version.
Requests remain strict: send new parameters or intent types only after the relay
advertises the corresponding capability. Missing optional data means unsupported
or not requested, never an empty list, zero count or unchanged revision.

Follow-up design constraints are recorded in
[the extension design note](buzz-v1-extension-design.md); they do not advertise
additional capabilities.

## Privacy and lifecycle

These typed relational frontiers are signer-private application state, **not
self-encrypted**. Database operators can see reading progress; ordinary Nostr
queries, search and moderator interfaces do not expose it. No public receipts
are emitted. Storage grows by touched contexts, not observed messages, and has
no fixed context-count ceiling.

Leaving/rejoining does not erase progress; revoked access hides it. Soft-deleted
channels are inaccessible, while hard channel deletion cascades their frontiers.
Deleting an account row cascades that actor's frontiers in the same community;
community erasure inventories both tables under the existing write fence. There
is no new public account export/reset endpoint. Operator-assisted erasure/export
must use the established authenticated operational process and explicitly scope
both community and actor; never equate the read-time horizon with data erasure.

There is no down migration, and disabling the API is not a rollback. A relay
built before 0056 that restarts with `BUZZ_AUTO_MIGRATE=true` (the Helm default)
refuses to start on the migrated schema. Whole-community deletion run from a
build before 0056 rejects the two new tables. A deletion approved on the
earlier schema and not yet fenced fails structural revalidation after 0056:
take pending approvals back through operator review before rollout, and do not
rewrite them.

Roll out disabled-by-default to controlled accounts after agent and human live
acceptance. Disabling the API unmounts it and leaves both tables in place: v1
clients lose access to read state and keep their unsent intents until it
returns. Neither setting changes NIP-RS state or how NIP-RS requests are
processed. While enabled, v1 requests count against the signer's existing
API-call quota and share the existing writer database pool with other relay
work.
