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
"max_channels":20,"max_intents":100,"eligible_kinds":[9,40002,45001,45003],
"capabilities":["follow","threads"],"max_threads":50}}
```

`capabilities` lists additions to the first v1 contract: `follow` for the
`follow` and `unfollow` intents, `threads` for `GET /me/threads`. Use one only
when it is listed. `retention_seconds` is how long an automatic thread follow
stays live without your involvement (see [Following](#following)).

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
Read state is for people: an agent account (one with an owner) gets 403 from
every endpoint, and the relay keeps no positions or follows for it.

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
| `unread` | an unread timeline message counts | an unread reply counts |
| `mentions` | unread timeline messages that tag you with `p`, or every unread one in a DM (at most 100) | unread replies that tag you with `p`, or every unread one in a DM (at most 100) |
| `read_through_id` | the anchor of the timeline's frontier, or null | the anchor of the thread's frontier, or null |
| `latest_id` | the last eligible timeline message to arrive, or null | the last reply to arrive |

The timeline holds top-level messages and replies carrying `broadcast=1`; a
broadcast reply also counts on its thread row. No other reply makes the channel
row unread or becomes its `latest_id`. `latest_id` is the anchor that reads its
scope: "mark channel read" sends a channel `mark_through` at the channel's
`latest_id`, plus one thread `mark_through` at each listed thread's `latest_id`
to clear those too. The channel `latest_id` counts any author and ignores read
progress. Null means no timeline message was recorded since 0074, not that the
channel is empty. `read_through_id` may name a message that has since been
deleted.

`threads` lists the threads you follow that have unread replies, newest
arrival first (then `root_id`), at most 25. Threads past
the 25th are omitted with their mentions, and nothing signals the omission:
after marking the listed threads read, a refresh can list the next ones. A
thread row's `unread` is always true in this version. No message bytes are
included.

A message is eligible when it is non-own, nondeleted, of the advertised
`eligible_kinds`; its author time plays no part. The same kinds alone define latest
activity, so an edit, reaction or diff (40008) neither makes a channel unread
nor moves it. Classify live arrivals with the advertised set, not a client copy.

### Following

Every top-level message counts on its channel row. A reply counts only in a
thread you follow. The relay records follows when it stores an eligible message,
in the same transaction:

- Its author has read through it: the timeline for a top-level message,
  otherwise the thread, which the author now follows. A broadcast reply reads
  only its thread.
- Each channel member it tags with `p` follows its thread (for a top-level
  message, the thread rooted at it).
- A reply also makes the root's author follow, and in a DM every member.

None of this applies to an agent account: it neither reads through its posts
nor follows, though its messages still make people follow.

A new follower starts just before the message that made them follow, so it is
unread; one who already follows keeps their position. Reading never follows:
a thread `mark_through` moves an existing follow and otherwise changes nothing.
`broadcast=1` does not make anyone follow. Follows start when the relay that
records them is deployed; there is no backfill. Desktop notification follows
and mutes do not affect these counts.

The `follow` intent follows a thread explicitly, from now: replies that arrived
before it are read. Following a thread you already follow keeps its position
and only makes the follow explicit.
`unfollow` stops counting it, and the stored message rules above no longer
follow you again, with two exceptions: posting in the thread follows you through
your post, and a mention follows you from just before it.

Counts are what the bounded walks could establish, and may undercount. A
message whose ancestry is unresolved is left out, as are messages beyond the
bounds (see [Bounds](#bounds-and-deployment)).

An automatic follow goes dormant after 30 days (`BUZZ_V1_RETENTION_SECONDS`)
without your involvement in the thread: posting there, being mentioned there,
a reply to your root or in your DM, marking it read, or following it. A dormant
thread is neither counted nor listed, but keeps its row and position, and any
of those involvements makes it live again at that position. Replies that do not
involve you never revive it. A follow made by the `follow` intent never goes
dormant; an unfollow ends that. Being mentioned, a reply to your root and DM
replies refresh a live follow at most once a day, or once a quarter of the
window when that is shorter, so dormancy is measured to that interval.

## Threads view

`GET /buzz/v1/me/threads?limit=50&cursor=<opaque>` lists the threads you follow
in joined, nondeleted channels, newest last reply first by arrival (then
`root_id`), read or not, at most 50 per page. Omit the cursor on the first
request and pass `next_cursor` back unchanged; null means the scan is done. A
thread with no reply yet is not listed.

```json
{"threads":[{"channel_id":"<uuid>","root_id":"<64-hex>","unread":false,
  "mentions":0,"read_through_id":"<64-hex>","latest_id":"<64-hex>"}],
 "next_cursor":"<opaque>"}
```

Each row has the sidebar thread row's fields and meanings plus its
`channel_id`, except that `unread` may be false and `mentions` is then 0. Pages
are separate snapshots: a thread that gets a new reply while you page moves
ahead of the cursor, so refresh from the first page after a change. Every
page probes each of your live follows, so its cost grows with how many threads
you are involved in, not with community size or your whole history.

## Fixed-operand writes

`POST /buzz/v1/me/read-state` accepts 1–100 independent intents:

```json
{"intents":[
 {"type":"mark_through","target":{"channel_id":"<uuid>"},"message_id":"<64-hex-event>"},
 {"type":"mark_through","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"},"message_id":"<64-hex-event>"},
 {"type":"follow","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"}},
 {"type":"unfollow","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"}}
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
anchor must be a top-level message or a broadcast reply; a thread anchor must
be the thread's root or one of its replies. `follow` and `unfollow` need a
thread target; a channel-only one is `invalid`. The frontier records the anchor as `read_through_id`
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
- A `follow` and the account's start use the database clock, so they can be
  off from relay arrivals by the skew between the two.

Counting walks forward from a position over indexes in arrival order: the
timeline's messages per channel and each thread's replies. A channel's
`latest_id` is exactly its last timeline arrival and a thread's its last reply
arrival, so marking through it reads everything counted. Mentions are indexed
by author time, which the relay accepts within 15 minutes of arrival, so a
mention walk starts 15 minutes of author time before the position: a mention
backdated further than that is unread but not counted as a mention.

A channel row is unread exactly when its `latest_id` is someone else's and
arrived after the timeline's position. Nothing earlier needs
checking: posting reads the timeline through the post, so an own `latest_id`
means everything before it is read, and an unread one is the anchor that clears
the row. A row is never unread without an anchor to mark.

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
- Counts stop at 100 per channel and per thread.
- A caught-up reader costs one index probe per channel and per followed
  thread; a reader who is behind walks only what arrived after their position.
- 50 rows per threads page; each page probes every live follow of the reader.
- DB statement/lock deadlines and HTTP read deadlines bound work; a write's intents
  and its row refresh share one eight-second deadline after admission. Limits are
  containment, not a production capacity claim.

Apply migrations 0056 through 0058 and 0074 (or the equivalent desired
schema). 0056 creates two empty private tables and no index on `events`, 0057
adds the nullable `personal_read_accounts.started_at` column, and 0058 adds the
nullable `personal_read_frontiers.through_message_id` column and drops
`threads_through_timestamp`. 0074 adds `events.read_timeline` (set at insert
for timeline messages), `thread_metadata.received_at` (set at insert for
eligible replies) and `personal_read_frontiers.unfollowed`, `explicit` and
`active_at` (existing rows start at migration time), plus four indexes:
`idx_events_read_timeline` for timeline walks, `idx_thread_metadata_read_arrival`
for thread walks, `idx_event_mentions_pubkey_channel_created` for mention
counts and `idx_personal_read_frontiers_active` for an actor's threads by
involvement. There is no backfill: messages stored before the new relay serves,
including by older relay processes still running after 0074, are on neither
arrival index, so they never count as unread or become `latest_id`. Ingest
writes a few frontier rows per eligible channel message (the author, and anyone
it makes follow), whether or not the API is enabled, so follows exist once it
is; there are no stored unread counters.

The column additions are catalog-only but briefly take an exclusive lock on
`events`, `thread_metadata` and `personal_read_frontiers`, bounded by the
migration's one-second `lock_timeout`; a busy writer fails the deployment,
which retries, rather than stalling ingestion. Building the indexes inside the
migration is bounded the same way, so on a database with data, add the columns
and prebuild the indexes before deploying, as for
[0049](thread-window-deployment.md). 0074 then skips both and only validates
the indexes. Run each statement on its own, retrying any that times out;
`CREATE INDEX CONCURRENTLY` cannot run in a transaction block:

```sql
SET lock_timeout = '1s';
ALTER TABLE events ADD COLUMN IF NOT EXISTS read_timeline BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE thread_metadata ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ;
ALTER TABLE personal_read_frontiers ADD COLUMN IF NOT EXISTS unfollowed BOOLEAN NOT NULL
    DEFAULT false CHECK (NOT unfollowed OR root_id <> ''::bytea);
ALTER TABLE personal_read_frontiers ADD COLUMN IF NOT EXISTS explicit BOOLEAN NOT NULL
    DEFAULT false;
ALTER TABLE personal_read_frontiers ADD COLUMN IF NOT EXISTS active_at TIMESTAMPTZ NOT NULL
    DEFAULT now();
CREATE INDEX CONCURRENTLY idx_personal_read_frontiers_active ON public.personal_read_frontiers
    (community_id, actor, active_at DESC) WHERE root_id <> ''::bytea;
CREATE INDEX CONCURRENTLY idx_thread_metadata_read_arrival ON public.thread_metadata
    (community_id, root_event_id, received_at DESC, event_id) WHERE received_at IS NOT NULL;
CREATE INDEX CONCURRENTLY idx_event_mentions_pubkey_channel_created ON public.event_mentions
    (community_id, pubkey_hex, channel_id, event_created_at DESC);
-- events is partitioned: an invalid parent, then each partition concurrently.
CREATE INDEX idx_events_read_timeline ON ONLY public.events
    (community_id, channel_id, received_at DESC, id) WHERE read_timeline AND deleted_at IS NULL;
-- For each partition p in
--   SELECT inhrelid::regclass FROM pg_inherits WHERE inhparent = 'public.events'::regclass:
CREATE INDEX CONCURRENTLY p_read_timeline ON p
    (community_id, channel_id, received_at DESC, id) WHERE read_timeline AND deleted_at IS NULL;
ALTER INDEX idx_events_read_timeline ATTACH PARTITION p_read_timeline;
```

The parent becomes valid once every partition's index is attached. Verify the
four like 0049's, against the definitions the migration expects; recover a
failed build the same way.

Use existing HTTP route/status/latency metrics for `/buzz/v1/me/sidebar` and
`/buzz/v1/me/read-state`, plus database pool/statement metrics. No payload,
actor, channel or frontier values should become metric labels. The measured
local seed is not a DAU/concurrency or p95/p99 production acceptance result.

## Compatibility and extension rules

Within `/buzz/v1`, clients must ignore unknown response object fields. The
shape is stable: existing fields, their types, status variants and what a
`mark_through` does remain. What the counts include is not yet: which messages
are unread or mentions, which threads are listed, and the bounds may change
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
are emitted. Storage grows by followed threads and posted-in channels, not
observed messages, and has no fixed context-count ceiling.

Leaving/rejoining does not erase progress; revoked access hides it. Soft-deleted
channels are inaccessible, while hard channel deletion cascades their frontiers.
Deleting an account row cascades that actor's frontiers in the same community;
community erasure inventories both tables under the existing write fence. There
is no new public account export/reset endpoint. Operator-assisted erasure/export
must use the established authenticated operational process and explicitly scope
both community and actor; never equate follow dormancy with data erasure.

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
