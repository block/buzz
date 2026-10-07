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
"max_channels":20,"max_intents":100,"max_contexts":20,"max_context_messages":100,
"max_thread_summaries":5,"eligible_kinds":[9,40002,45001,45003]}}
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
`account`, `channels`, and `next_cursor`. Omit the cursor on the first request.
Each channel includes identity/name/type, archived and hidden flags, `unread`,
`attention`, `latest_message_id` (the last eligible message to arrive: the
row's read anchor, see [Order](#order)), `latest_message_at` (the greatest
author time among eligible messages, in seconds: display activity, not
necessarily that message's own time; null exactly when the ID is null),
`latest_message_complete`, and `threads`. Only joined, nondeleted channels are
listed. Hidden/archived presentation remains client-owned. Each page has a
writer-consistent snapshot; separate pages do not share a snapshot, and an
unfinished traversal cannot prove channel removal.

`GET /buzz/v1/me/sidebar?channel_ids=<uuid>,<uuid>` refreshes 1–20 unique
channels in one snapshot, ordered by ID with `next_cursor: null`. It cannot be
combined with `limit` or `cursor`. A requested ID absent from the result was not
a joined, nondeleted, accessible sidebar row at that snapshot: remove its row.
Absence says nothing else about access to an open channel.

`threads` lists unread threads in the row, newest unread reply first:

```json
{"items":[{"root_id":"<64-hex>","unread":{"status":"exact","value":2},
  "latest_reply_id":"<64-hex>","latest_reply_at":1700000000}],"complete":true}
```

Items are canonical roots with unread replies that count (see below), ordered
by `latest_reply_at` descending, then `root_id`; at most 5. `latest_reply_id` is
the last such reply to arrive (equal arrivals prefer the smaller ID), so a
thread `mark_through` at it reads every reply listed; `latest_reply_at` is its
author time. Replies that do not count are filtered out before the count, the
latest reply and the cap are chosen. `unread` uses the row's
definition; every counted reply is also attention, so items carry no separate
attention count. `complete=true` means the unread window was exhausted, no
evidence had unresolved ancestry, unusable tags or undecided membership, and no
thread was omitted; then item unread counts sum to the row's unread replies.
Otherwise the list is a cut of observed evidence and counts may be lower bounds
or unknown. No message bytes are included.

Counts have exactly three representations:

```json
[{"status":"exact","value":0},{"status":"at_least","value":7},{"status":"unknown"}]
```

Only exact zero proves absence. Unknown has no numeric value. A message is
eligible when it is non-own, nondeleted, of the advertised `eligible_kinds` and
inside the horizon. The same kinds alone define latest activity, so an edit,
reaction or diff (40008) neither makes a channel unread nor moves it. Classify
live arrivals with the advertised set, not a client copy.

An eligible message beyond the matching context frontier counts as unread for
the first reason that holds:

| `reason` | Holds when |
|---|---|
| `direct` | its channel is a DM |
| `mention` | it tags the actor with `p` |
| `conversation` | it is a reply, and the actor wrote its direct parent or has a reply to that same parent, in that channel |
| `broadcast` | it carries `broadcast=1` |
| null | it is top-level |

A reply with no reason does not count: it is not unread and appears in no
thread list. `unread` counts the messages that count; `attention` is the subset
with a reason. Conversation membership uses only the actor's live eligible
messages (a deleted parent proves nothing; a surviving reply to it still does),
looks at the direct parent only (owning the root or replying elsewhere in the
thread proves nothing), and is independent of read progress and retention. A
reply whose membership is undecided is left out, and the row's counts become
lower bounds or unknown. The sidebar counts a broadcast reply without asking
which of the two reasons applies. This is not Desktop notification policy:
follows and mutes do not affect these counts.

The unread horizon defaults to 30 days (`BUZZ_V1_RETENTION_SECONDS`) and is
measured in author time (`created_at`): a message counts while its author time
is at or after `account.cutoff_ms`. It filters unread/attention, not latest
activity, event storage or frontier state. Frontiers use a different clock,
relay arrival (see [Order](#order)). Three consequences:

- A message accepted late with an author time beyond the horizon (an import, a
  backfill, a long-offline sender) is excluded under the current horizon,
  however recently the relay accepted it. It can still be latest when the
  horizon holds no message.
- Unread expires at author time plus the horizon, so a future-dated author
  time extends how long a message counts.
- A context's unread set is two tests, not one range: arrived after the
  frontier, and author time at or after the cutoff.

A later configuration expansion can change counts without having lost progress.
Latest activity is independent of actor and frontiers. A null latest ID proves
an empty eligible history only when `latest_message_complete=true`.

## Explicit contexts

`GET /buzz/v1/me/read-state?targets=<URL-encoded-JSON-array>` accepts up to 20
contexts and 100 total concrete message selectors. It is not event history or a
global export of frontiers. Example decoded `targets`:

```json
[{"target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"},
  "message_ids":["<64-hex-event>"]}]
```

Omitting `root_id` selects the channel timeline. The result contains `account`
and one `contexts` entry per request entry, in order. Context status is
`available` (with `messages`), `unknown`, or `unavailable`. A thread context's
frontier includes any whole-channel cut. Message status is `read`, `not_counted`,
`unread` (with `reason`), `unknown`, or `unavailable`. Wrong-context, missing
and forbidden selectors share unavailable. Status is decided in this order:
ancestry and context; eligibility (`not_counted` for own, deleted, other kinds
and outside the horizon); the frontier (`read`, with no membership lookup); then
the reason. A reply past the frontier with no reason is `not_counted` when it is
proven outside the actor's conversations and `unknown` when membership is
undecided. A broadcast reply whose membership is undecided is `unread` with
reason `broadcast` and may report `conversation` on a later request.
Conversation bytes must still come from the existing Nostr path.

## Fixed-operand writes

`POST /buzz/v1/me/read-state` accepts 1–100 independent intents:

```json
{"intents":[
 {"type":"mark_through","target":{"channel_id":"<uuid>"},"message_id":"<64-hex-event>"},
 {"type":"mark_through","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"},"message_id":"<64-hex-event>"},
 {"type":"mark_channel_read","channel_id":"<uuid>","message_id":"<64-hex-event>"}
]}
```

Each intent commits atomically and returns its own `applied`, `blocked` or
`invalid` outcome. An ambiguous timeout/storage failure returns
`{"status":"unknown","retryable":true}`. Earlier committed outcomes survive
later failures. `projection_status` is `not_requested`; a successful write does
not assert a client has refreshed. Retry the same operands, never substitute
latest. Keep pending intent durably on the client until its outcome is resolved.

A mark-through validates a fixed message and advances its context's monotone
frontier to that message's relay arrival (see [Order](#order)). Channel and
thread frontiers never inherit in either direction. Opening a view is not
itself a reading action; client dwell/focus policy determines when to send an
actual observed anchor. Old or deleted valid anchors may advance a frontier.

`mark_channel_read` is the one whole-channel cut: it advances the channel
timeline and every thread in that channel, including unlisted ones, through the
anchor's arrival. The anchor must be an accessible eligible-kind message in
the channel, top-level or reply, deleted or not; ancestry is not checked. A
reply is read at or below the greater of its thread frontier and this cut. An
anchor that no longer exists is `blocked`. `latest_message_id` is the anchor
that reads the whole row. A null ID with `latest_message_complete=false` does
not prove empty history; it only leaves the client without an anchor. Thread
marks and channel `mark_through` never set the cut.

### Order

A frontier is the relay arrival time (`events.received_at`) of the message a
context was read through, never its author time, which the sender chooses and
the relay accepts up to 15 minutes either way. Everything that arrived at or
before the anchor is read, whatever its author time. A message that arrives
later is unread even when backdated, and a future-dated anchor reads nothing
that arrives after it.

The order is the relay's and is not exposed: no response carries a frontier,
and no field lets a client compute what a mark will cover. Send the anchors the
user actually saw, let the relay take the greatest, and ask a context which
messages are read. Do not compare author times, IDs or local receipt order to
drop one pending anchor in favor of another.

Arrival is the accepting relay process's clock, read just before the insert, at
microsecond resolution. Three limits follow, none of which strands a badge:

- It is not commit order. An insert that commits after a later-stamped message
  was already marked read lands read.
- Relay processes with different clocks can stamp out of true order by their
  skew.
- Messages with the identical stamp are read together.

`latest_message_id` is the last message to arrive among those the unread count
examined: the 4,096 most recent events by author time inside the horizon. So
marking through it reads everything counted. When the horizon holds no message,
it is the last to arrive among the channel's 256 most recent events.

There is no import of earlier client read state: an account starts with no
frontiers, and the horizon bounds what that can show as unread. Manual unread
remains device-local.

## Bounds and deployment

- 20 sidebar rows, 100 intents, 20 contexts / 100 selectors per request.
- 64 KiB write body; 16 KiB context URL; 1 MiB serialized API response.
- 4096 raw events per channel inside the horizon plus one exhaustion sentinel,
  before eligibility.
- Latest activity probes 256 events plus a sentinel; long ineligible tails may
  leave latest incomplete even when unread is exact.
- Tag documents over 8192 bytes or malformed relevant tags yield uncertainty.
  Compact boolean facts cross the database boundary, never raw tag payloads.
- Conversation membership: at most 1024 unique parents per request, with a
  500 ms savepoint budget. The lookup is exact, so its work grows with the
  replies under each parent. Past either bound a reply is undecided, never
  absent: counts become lower bounds or unknown and a context reports `unknown`.
- DB statement/lock deadlines and HTTP read deadlines bound work; writes use a
  shared eight-second intent-processing deadline after admission. Limits are
  containment, not a production capacity claim.

Apply migration 0056 (or the equivalent desired schema). It creates two empty
private tables and no index on `events`: both sidebar scans are served by the
existing `idx_events_community_channel_created`. No new per-message ingest write
path or stored unread counters are introduced.

Use existing HTTP route/status/latency metrics for `/buzz/v1/me/sidebar` and
`/buzz/v1/me/read-state`, plus database pool/statement metrics. Inspect exact /
lower-bound / unknown proportions in controlled acceptance captures; no payload,
actor, channel or frontier values should become metric labels. The measured
local seed is not a DAU/concurrency or p95/p99 production acceptance result.

## Compatibility and extension rules

Within `/buzz/v1`, clients must ignore unknown response object fields. Existing
required fields, status variants and their meanings remain stable; additive
fields do not authorize silently changing `attention` or frontier semantics.
Breaking changes require an explicitly negotiated contract or a new API version.
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

Migration 0056 must be applied before this relay serves, enabled or not: started
without it and with auto-migration off, the relay stops before readiness. There
is no down migration, and disabling the API is not a rollback. A relay built
before 0056 that restarts with `BUZZ_AUTO_MIGRATE=true` (the Helm default)
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
