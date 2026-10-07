# Private read-state accessory API

`BUZZ_V1_ENABLED=true` opts into `/buzz/v1`; it is disabled by default.
Conversation history, live events, edits and deletion remain Nostr-authoritative.
This API neither replaces Nostr reads nor writes artificial signed events.
Legacy NIP-RS continues unchanged, but does not synchronize with these tables.

## Discovery and identity

On a known community host, NIP-11 (`GET /` with `Accept:
application/nostr+json`, or `GET /info`) includes `buzz_v1` only when enabled:

```json
{"buzz_v1":{"version":1,"base_path":"/buzz/v1","max_channels":20,"max_intents":100,
"max_contexts":20,"max_context_messages":100,"eligible_kinds":[9,40002,45001,45003]}}
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
`channels` and `next_cursor`. Omit the cursor on the first request. Only joined,
nondeleted channels are listed. Hidden/archived presentation remains
client-owned. Each page has a writer-consistent snapshot; separate pages do not
share a snapshot, and an unfinished traversal cannot prove channel removal.

`GET /buzz/v1/me/sidebar?channel_ids=<uuid>,<uuid>` refreshes 1–20 unique
channels in one snapshot, ordered by ID with `next_cursor: null`. It cannot be
combined with `limit` or `cursor`. A requested ID absent from the result was not
a joined, nondeleted, accessible sidebar row at that snapshot: remove its row.
Absence says nothing else about access to an open channel.

Each row is one read scope for the channel timeline plus one per followed
thread with an unread reply:

```json
{"channel_id":"<uuid>","name":"general","channel_type":"stream","archived":false,
 "hidden":false,"unread":true,"mentions":2,"read_through_id":"<64-hex>",
 "threads":[{"root_id":"<64-hex>","unread":true,"mentions":1,
             "read_through_id":"<64-hex>","latest_id":"<64-hex>"}]}
```

- `unread`: a message in that scope arrived after your read position. The
  channel scope is the timeline only (top-level messages and broadcast
  replies). A reply in a thread never makes the channel unread; it shows on
  its thread row.
- `mentions`: exact, uncapped count of unread messages in that scope that tag
  you with `p`. A thread mention counts on the thread row, not the channel.
- `read_through_id`: the last message in that scope, in display order, at or
  before your read position, or null when there is none. Draw the "new"
  divider below it and open the view there.
- `latest_id` (threads only): the last reply in display order. Fetch it by ID
  for a preview; marking the thread through it reads every reply above it,
  late arrivals included.

`threads` lists every followed thread with an unread reply, by `latest_id`
(newest author time first), then by `root_id`. There is no cap. A badge or a threads
page is computed on the client from these rows. Clients receive message IDs
only; positions and timestamps stay relay-internal. No message bytes are
included.

A message counts when it is nondeleted and of the advertised `eligible_kinds`,
so an edit, reaction or diff (40008) never makes a scope unread. Classify live
arrivals with the advertised set, not a client copy. Your own messages need
no rule: posting marks you read through what you post (see
[Ingest](#ingest-posting-and-following)).

## Ingest: posting and following

The relay updates read state when it stores an eligible message:

- **Posting marks read.** The author's position moves through their own
  message in each scope it shows in: the timeline for a top-level message or a
  broadcast reply, the thread for a reply.
- **Following is a relay-side thread row.** You follow a thread when you reply
  to it, when someone replies to your top-level message, when a message
  mentions you (the thread it is in, or the thread rooted at it), in a DM when
  any member replies, or with a `follow` intent. A new follow starts just
  before the message that caused it, so that message is unread.
- **Unfollow is sticky.** `unfollow` keeps the row with `following=false`, so
  later replies do not follow you again. Your own reply, `follow`, or a new
  mention does. A mention catches the thread up to just before itself: the
  replies from while you were unfollowed stay read, and the mention shows.

An account starts caught up at its first read intent (`started_at`): every
position is floored there, so history from before then never counts, and
nothing counts before the first intent. Joining a channel starts that channel
caught up, its followed threads included: a follow that survived leaving does
not bring back replies from your absence.

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
ancestry and context; eligibility (`not_counted` for own, deleted, other kinds,
or before the account started); the frontier (`read`, with no membership lookup); then
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
 {"type":"mark_channel_read","channel_id":"<uuid>","message_id":"<64-hex-event>"},
 {"type":"follow","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"}},
 {"type":"unfollow","target":{"channel_id":"<uuid>","root_id":"<64-hex-root>"}}
]}
```

Each intent commits atomically and returns its own `applied`, `blocked` or
`invalid` outcome. An ambiguous timeout/storage failure returns
`{"status":"unknown","retryable":true}`. Earlier committed outcomes survive
later failures. `projection_status` is `not_requested`; a successful write does
not assert a client has refreshed. Retry the same operands, never substitute
latest. Keep pending intent durably on the client until its outcome is resolved.

A mark-through validates a fixed message and reads everything displayed at or
before it in its scope (see [Order](#order)). Channel and thread positions never
inherit in either direction. Opening a view is not itself a reading action;
client dwell/focus policy determines when to send an actual observed anchor.
Old or deleted valid anchors may advance a position. Marking a thread you do
not follow changes nothing.

`mark_channel_read` is the one whole-channel cut: it reads the channel
timeline and every thread in that channel, including unlisted ones, through
everything in the channel displayed at or before the anchor. The anchor must be
an accessible eligible-kind message in the channel, top-level or reply, deleted
or not; ancestry is not checked. A reply is read at or below the greater of its
thread position and this cut. An anchor that no longer exists is `blocked`.
Thread marks and channel `mark_through` never set the cut.

`follow` and `unfollow` take a thread target; one without `root_id` is
`invalid`. Following starts caught up at the thread's last reply, including
when you follow again after unfollowing.

### Order

A position is a relay arrival time (`events.received_at`), never author time,
which the sender chooses and the relay accepts up to 15 minutes either way. A
message that arrives after your position is unread even when backdated, and a
future-dated message cannot read anything that arrives after it.

Clients see display order (author time, then ID) and name only message IDs. A
mark therefore reads everything displayed at or before its anchor in that
scope: the position becomes the last arrival among them. A late arrival shown
above the newest message is read when you mark that message, so marking the
bottom of a view always clears its badge. The cost, only when a late arrival
sits above the anchor, is that a message displayed below the anchor but arrived
before the late one is read too. With no late arrival, a mark reads exactly
what arrived by the anchor, and a message shown below it stays unread.

Positions are never exposed. `read_through_id` is the last message displayed at
or before the position; in the rare late-arrival case an unread message can sit
above that divider, while the row's `unread` stays correct.

Arrival is the accepting relay process's clock, read just before the insert, at
microsecond resolution. It is not commit order, relay processes with different
clocks can stamp out of true order by their skew, and messages with the
identical stamp are read together.

There is no import of earlier client read state. Manual unread remains
device-local.

## Bounds and deployment

- 20 sidebar rows, 100 intents, 20 contexts / 100 selectors per request.
- 64 KiB write body; 16 KiB context URL; 1 MiB serialized API response.
- A channel whose last timeline arrival (`channels.last_timeline_received_at`)
  is at or before the position is read with one comparison, without walking
  past replies. Otherwise `unread` is an existence probe that stops at the
  first message past the position, which also rules out a deleted latest
  message. Counting is forward
  from the position: work grows with what is unread, not with history.
- Mention counts walk only your mentions in that scope past the position
  (`idx_event_mentions_scope_received`).
- A followed thread with nothing new is skipped by comparing the root's
  `thread_metadata.last_reply_received_at` with the thread position, without
  reading replies.
- DB statement/lock deadlines and HTTP read deadlines bound work; writes use a
  shared eight-second intent-processing deadline after admission. Limits are
  containment, not a production capacity claim.

Apply migrations 0056 and 0057 (or the equivalent desired schema). 0056
creates two private tables. 0057 adds `personal_read_accounts.started_at`
(NULL for accounts from 0056, which start at their next read intent),
`personal_read_frontiers.following`, `thread_metadata.last_reply_received_at`,
`channels.last_timeline_received_at` and
`event_mentions.received_at`/`root_id` (NULL for rows from before it, which no
position can reach), and one index on `event_mentions`. It also drops
`idx_thread_metadata_root`, whose columns lead `idx_thread_metadata_window`
(0049). Ingest writes the author's own position and follow rows in the same
transaction as the message.

Like 0049, 0057 bounds lock waits and statement time, so it fails
instead of blocking `event_mentions` writes while building its index on a
populated table. Brownfield deployments prebuild the index first. Adding
nullable columns is a catalog-only change; `CREATE INDEX CONCURRENTLY` cannot
run inside a transaction block:

```sql
ALTER TABLE public.event_mentions
    ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS root_id BYTEA;
CREATE INDEX CONCURRENTLY idx_event_mentions_scope_received
    ON public.event_mentions (community_id, pubkey_hex, channel_id, root_id, received_at);
```

Verify it with the `pg_index` query in
[thread-window-deployment.md](thread-window-deployment.md), using
`idx_event_mentions_scope_received`. All flags must be `true`, and the
definition must be exactly:

```text
CREATE INDEX idx_event_mentions_scope_received ON public.event_mentions USING btree (community_id, pubkey_hex, channel_id, root_id, received_at)
```

Migration 0057 then skips the build and validates the catalog shape again. It
rejects an invalid or differently defined index. Recover the same way as for
0049: `DROP INDEX CONCURRENTLY`, then prebuild again.

Its transactional `DROP INDEX IF EXISTS idx_thread_metadata_root` needs a brief
`ACCESS EXCLUSIVE` lock on `thread_metadata`, bounded by the same lock budget.
On a busy database, drop it beforehand, also outside a transaction:

```sql
DROP INDEX CONCURRENTLY IF EXISTS public.idx_thread_metadata_root;
```

0057 still takes brief exclusive locks to add columns to the two read-state
tables, `thread_metadata`, `channels` and `event_mentions`. If a long reader holds one of them past the
lock budget, startup fails and can be retried; ingestion does not queue behind
it.

During a rolling deploy, relay processes still on the old code write
`event_mentions` rows without `received_at`/`root_id` and do not mark posting
read. Those mentions never count, an author's own message from such a
process can show unread to them until they next read the scope, and a
channel or thread whose only new messages came through such a process shows
read until its next message through new code (the arrival columns are not
updated).

Use existing HTTP route/status/latency metrics for `/buzz/v1/me/sidebar` and
`/buzz/v1/me/read-state`, plus database pool/statement metrics. No payload,
actor, channel or position values should become metric labels.

## Compatibility and extension rules

Within `/buzz/v1`, clients must ignore unknown response object fields. Existing
required fields, status variants and their meanings remain stable; additive
fields do not authorize silently changing `unread`, `mentions` or position
semantics.
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
are emitted. Storage grows by touched channels and followed threads, not observed
messages, and has no fixed ceiling.

Leaving/rejoining does not erase progress; revoked access hides it. Soft-deleted
channels are inaccessible, while hard channel deletion cascades their frontiers.
Deleting an account row cascades that actor's frontiers in the same community;
community erasure inventories both tables under the existing write fence. There
is no new public account export/reset endpoint. Operator-assisted erasure/export
must use the established authenticated operational process and explicitly scope
both community and actor; never equate the read-time horizon with data erasure.

Migrations 0056 and 0057 must be applied before this relay serves, enabled or
not: started without them and with auto-migration off, the relay stops before readiness. There
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
