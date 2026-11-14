# Private activity and read-state accessory API

`BUZZ_V1_ENABLED=true` opts into `/buzz/v1`; it is disabled by default.
Conversation history, live events, edits and deletion remain Nostr-authoritative.
This API neither replaces Nostr reads nor writes artificial signed events.
Legacy NIP-RS continues unchanged, but does not synchronize with these tables.

## Discovery and identity

On a known community host, NIP-11 (`GET /` with `Accept:
application/nostr+json`, or `GET /info`) includes `buzz_v1` only when enabled:

```json
{"buzz_v1":{"version":2,"base_path":"/buzz/v1","retention_seconds":2592000,
"max_rows":500,"max_intents":100,"eligible_kinds":[9,40002,45001,45003]}}
```

Version 2 is the contract below. Version 1 (`/me/sidebar`) was never used by a
released client and is gone; a client that finds another version shows no read
state. Use the requesting origin plus this relative prefix. Discovery is a configured
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

## The list

`GET /buzz/v1/me/activity` returns one person's conversations, threads and
mentions as rows of one shape, newest activity first. It is what a sidebar, an
inbox and a threads view read, each with its own filter.

| Parameter | Meaning |
|---|---|
| `kinds` | Comma-separated: any of `channel`, `dm`, `solo`, `thread`, `mention`. Default: all. |
| `unread=true` | Only unread rows. Applies to listing (with or without `before`). |
| `limit` | Rows per page, 1–500, default 100. |
| `before` | The `next` cursor of the previous page. Valid only with the `kinds` and `unread` it came from. |
| `since` | The `marker` of an earlier read: only what changed. Not combinable with `before`, `ids` or `limit`. |
| `ids` | 1–100 comma-separated row ids to refresh. A UUID names a conversation; a 64-hex id names a thread, a mention, or both. Not combinable with the paging parameters. |

```json
{"rows":[{"kind":"thread","id":"<root id>","channel_id":"<uuid>","root_id":"<root id>",
   "activity":{"at":1731550000,"id":"<64-hex>","pubkey":"<64-hex>"},
   "preview":{"id":"<64-hex>","pubkey":"<64-hex>","created_at":1731549990,
     "text":"first 256 characters","edited":false},
   "unread":true,"mentions":1,"read_through_id":"<64-hex>|null",
   "state":"default","reply_count":4}],
 "removed":[{"kind":"channel","id":"<uuid>"}],
 "totals":{"channel":{"unread":2,"mentions":1},"dm":{"unread":0,"mentions":0},
   "solo":{"unread":0,"mentions":0},"thread":{"unread":1,"mentions":1},
   "mention":{"unread":1,"mentions":1}},
 "next":"<cursor>|null","marker":"<opaque>|null","reset":false}
```

### Rows

| `kind` | `id` | Listed when | `preview` |
|---|---|---|---|
| `channel`, `dm`, `solo` | channel UUID | you are a current member of the channel. `dm` is a DM channel, `solo` a channel labelled `buzz.solo`, `channel` the rest | newest top-level message |
| `thread` | root event id | the thread has a reply, its channel is one you are in, and you are part of it: you replied, someone replied to your message, a reply tagged you, within the last 30 days; or you follow it, muted it, or flagged it unread | newest reply |
| `mention` | the mentioning event id | a message of an eligible kind in a channel you are in tags you with `p`, arrived within `retention_seconds`, is not deleted and is not yours | the message itself |

`root_id` is present on thread rows and on mentions inside a thread.
`reply_count` is present on thread rows. Names, types, membership lists and
archived state come from Nostr channel metadata, not this API.

- **`activity`** is the newest message of the row: for a conversation, at any
  depth. `activity.at` is the relay's arrival time in Unix seconds and is the
  sort key for every kind. A conversation with no message carries its creation
  time and a null `id`.
- **`preview`** is what to show. For a conversation it follows the channel
  window's rule: a message with no parent, or a depth-1 reply broadcast to the
  channel. A message tagged `["audience","agents"]` is never a preview. The
  latest edit of a conversation's preview is applied (`edited`); a thread's
  preview is the reply as stored. `preview.created_at` is the author's time,
  for display only. A deleted message is never returned: the row falls back to
  the previous one, or `preview` is null.
- **`unread`**
  - conversation: its preview is someone else's and arrived after your read
    position, or you flagged it unread. A reply that is not broadcast never
    makes a conversation unread.
  - thread: its newest reply is someone else's and arrived after your read
    position, while the thread is live for you: `following`, or you were last
    part of it (posted, were replied to, were tagged) within 30 days. Or you
    flagged it unread.
  - mention: it arrived after the read position of the channel timeline or
    thread it is in. Marking that scope read reads its mentions.
  - A `muted` conversation or thread is never unread. Its `mentions` still
    count, and a mention row inside it is still unread: a mention gets through
    a mute.
- **`mentions`** counts unread messages in the row's own scope that tag you: a
  conversation counts top-level messages, a thread its replies. It stops at 99.
  A DM does not count every message as a mention; badge DMs on `unread`.
- **`read_through_id`** is the message the scope was last marked through, or
  null. It may name a message deleted since.
- **`state`** is `default`, `following` or `muted`.

Nothing is unread before your account starts. Your first request to this API
starts it; every read position is floored at that moment, so a scope you never
marked counts only what arrived afterwards.

### Order and paging

Rows sort by `activity.at` descending (compared at microsecond resolution),
then conversations before threads before mentions, then `id`. `next` is null on
the last page. A page is cut short of `limit` when it would exceed the response
size; follow `next`.

Pages are separate snapshots: a row that moves up while you page can be missed
or seen twice. Page to fill a view; use `since` to stay current.

### Totals

`totals` counts unread rows and unread mentions per kind over the whole set,
whatever the filter and page. It comes with a first page (no `before`) and with
every `since` read, and is null otherwise. `totals.mention` is the number of
unread mentions (both fields); mention counts stop at 99.

### Staying current

A first page and every `since` read return a `marker`. `GET
/me/activity?since=<marker>` returns the rows that changed since then, the rows
that left (`removed`), fresh `totals` and a new marker. Replace rows by
`(kind, id)`. A read of `ids` also returns a marker. A `since` read may repeat a row that did not change (rows changed
in the 30 seconds before the marker are sent again).

`since` covers:

- every change you made: marks, manual unread, state changes, and the read
  position your own post moved;
- a new message in a row's scope, an edit of a conversation's preview, a change
  of a thread's reply count;
- joining or leaving a channel. `removed` names the conversation; drop its
  thread and mention rows with it (they carry `channel_id`).
- a thread your own change took off the list (a state set back to `default`
  or a flag cleared on a thread you are not otherwise part of). `removed`
  names the thread.

It does not report a deletion that changes only a count, or a row whose only
change is that it aged out. `reset: true` means the marker is older than 30
days or too much changed (more than 1,000 conversations, threads or
mentions): discard the list and read
page one. `since` ignores `unread`; it honours `kinds`.

A WebSocket connection admitted as you also receives your own changes as
they commit, from any of your devices:

```json
["BUZZ","activity",{"rows":[ ... ]}]
```

The rows are the ones the `POST` below returned. The push is best effort and
unordered relative to reads; treat it as a hint and let `since` correct it.
New messages are not pushed this way: they already arrive as events.

## Writes

`POST /buzz/v1/me/read-state` accepts 1–100 independent intents:

```json
{"intents":[
 {"type":"mark_through","target":{"channel_id":"<uuid>"},"message_id":"<64-hex>"},
 {"type":"mark_through","target":{"channel_id":"<uuid>","root_id":"<64-hex>"},"message_id":"<64-hex>"},
 {"type":"mark_unread","target":{"channel_id":"<uuid>","root_id":"<64-hex>"}},
 {"type":"set_state","target":{"channel_id":"<uuid>"},"state":"muted"}
]}
```

A target without `root_id` is the channel's timeline; with it, that thread.
The two keep independent read positions.

- **`mark_through`** moves the read position forward to that message's arrival
  and clears a manual unread. It never moves back: an anchor that arrived at or
  before the current position changes only the flag. A channel anchor is a
  message of the channel's timeline (top-level, or a broadcast reply); a thread
  anchor is the thread's root or one of its replies. To mark a row read, send
  its `preview.id`.
- **`mark_unread`** flags the scope unread until the next `mark_through`.
- **`set_state`** sets `default`, `following` or `muted`.

Posting a message moves your own read position to it, in the thread and, for a
top-level or broadcast message, in the channel. No intent is needed. It does not
clear a manual unread; only `mark_through` does.

The response is `{"outcomes":[...],"rows":[...]}`. Each intent commits on its
own and returns `applied`, `blocked` (missing or forbidden, deliberately the
same) or `invalid`, in request order. An ambiguous timeout or storage failure
returns `{"status":"unknown","retryable":true}`; retry the same operands, never
substitute a newer message. Earlier committed outcomes survive later failures.
`rows` holds the updated conversation and thread rows of the applied targets,
in the list's row shape; a target in a channel you have not joined, or a thread
the list would not show, has no row.
Mention rows in a marked scope are not returned: flip them locally or read
`since`. `rows` is omitted when the rows could not be read within the request's
deadline after the writes committed.

A read position is the relay arrival time of its anchor, never the author's
time, which the sender chooses. The order is the relay's: send the anchors the
user actually saw and let the relay take the greatest. Arrival is the accepting
relay process's clock: it is not commit order, processes can differ by their
clock skew, and messages with the identical stamp are read together.

## Stored state

| Table | One row per | Holds |
|---|---|---|
| `channel_heads` | conversation | newest message (arrival, id, author), newest top-level message with its text and latest edit |
| `thread_heads` | thread with a reply | newest reply with its text, reply count |
| `personal_read_frontiers` | person and scope | read position, state, manual unread, when the person was last part of the thread, their change counter at their last write |
| `personal_read_accounts` | person | when unread tracking started, their change counter |
| `event_mentions` | person tagged in an event | now also the event's arrival and thread root |

The event log is the truth. The two shared tables are caches of it that prove
their own freshness: each row records the newest stored event it accounts for
(`seen_at`), and a read asks the log, with one index probe per row, whether
anything newer exists. A row that is missing, behind, or names a deleted
message is rebuilt from history before it is returned. So nothing has to be
backfilled, and a lost write is found by the next read (two exceptions are
under Limits).

- **On a message.** The storing transaction only records in memory what it
  stored. After it commits, a background writer in the same process coalesces
  per channel and writes the shared rows about 25 ms later, in its own
  transaction. A busy channel costs one write per interval per relay process,
  and no message transaction holds or waits for a shared row. The same write
  moves the author's read position and marks the thread as one the parent's
  author and the people tagged are part of.
- **On a read.** A first page, a `since` read and a read of named rows check
  the person's joined channels and live threads and rebuild what is behind, at
  most 256 rows per read and four rebuilds at once per process; a read that
  cannot get a turn serves what is stored and the next read tries again.
- **On an intent.** The person's account row is locked and its counter
  incremented; the scope's row is stamped with it. Counter order is therefore
  commit order, which is what makes the personal half of `since` exact.

The marker is `{counter, database time}`. A `since` read returns the person's
rows with a higher counter, plus shared rows, memberships and mentions changed
after the marker's time less 30 seconds.

### Limits

- A background write is lost when its process exits before it runs, or when
  eight attempts fail (a row held by a rebuild, a database error). After
  failed attempts the rows are marked unverified, so the next read rebuilds
  them. After a process exit the next read still finds the gap, unless another
  event in that channel was written first or the lost message carries an
  author time older than one already seen (a slow clock, up to 15 minutes);
  then the row is right again from the next message on.
- People's own rows are written separately from the shared rows and before
  them, and nothing rebuilds them. If that write is lost, the author may see
  their own message as unread, and a person replied to or tagged in it is not
  added to its thread (the mention row is unaffected).
- A message with an author time ahead of the relay's clock is checked for only
  once that time has passed.
- Threads you neither follow, muted nor flagged leave the list 30 days after
  you were last part of them.
- Mention rows written before this migration have no arrival time and are not
  listed or counted. Filling them is a one-off maintenance job, not part of
  this change.
- Mention counts examine the person's 2,000 newest mentions since their floor.
- The rebuild examines a conversation's newest 512 events and a thread's newest
  512 replies. A conversation whose newest 512 events hold no eligible message
  shows no activity until a new message arrives.
- A conversation's `activity` may name a message deleted since; its `preview`
  never does.
- At most 64 people tagged by one message, and 1,024 people per channel per
  write interval, are marked part of a thread.
- An unfollowed thread stops being unread 30 days after you were last part of
  it, without a `since` change.
- 64 KiB write body; 1 MiB response. DB statement and lock deadlines and an
  eight-second request deadline bound the work.

Apply migrations 0089 and 0090 (or the equivalent desired schema). 0089 creates
`channel_heads` and `thread_heads` empty, adds nullable or defaulted columns to
the three existing tables and makes `personal_read_frontiers.through_timestamp`
nullable. 0090 builds the two indexes on those columns inline and verifies
them. `event_mentions` is large: a deployment that applies migrations out of
band should apply 0089, build both indexes with `CREATE INDEX CONCURRENTLY`
(the statements are in 0090's header), then apply 0090, which skips an index
that already exists. The previous release never reads or writes the new columns, and its
inserts keep working against the migrated schema.

## Compatibility

Clients must ignore unknown response fields and unknown row kinds. Requests
are strict: unknown parameters and intent types are rejected, so send a new one
only after the relay advertises a version that has it. The WebSocket frame is
additive: a client that does not know `BUZZ` frames ignores them.

Not in this version: a window of rows around a given message, search cursors,
and synchronisation with NIP-RS.

## Privacy and lifecycle

These typed relational rows are signer-private application state, **not
self-encrypted**. Database operators can see reading progress; ordinary Nostr
queries, search and moderator interfaces do not expose it. No public receipts
are emitted. Storage grows by touched contexts, not observed messages, and has
no fixed context-count ceiling.

Leaving/rejoining does not erase progress; revoked access hides it. Soft-deleted
channels are inaccessible, while hard channel deletion cascades their frontiers.
Deleting an account row cascades that actor's frontiers in the same community;
community erasure inventories these tables, `channel_heads` and `thread_heads`
included. The two shared tables are written only through the relay's
community-admitted transactions and carry no write-fence trigger. The two shared tables have no foreign
keys: a hard-deleted channel's rows stay until community erasure and are never
read, because every read starts from current membership. There
is no new public account export/reset endpoint. Operator-assisted erasure/export
must use the established authenticated operational process and explicitly scope
both community and actor; never equate the read-time horizon with data erasure.

There is no down migration, and disabling the API is not a rollback. A relay
built before 0089 that restarts with `BUZZ_AUTO_MIGRATE=true` (the Helm default)
refuses to start on the migrated schema. Whole-community deletion run from a
build before 0089 rejects the two new tables. A deletion approved on the
earlier schema and not yet fenced fails structural revalidation after 0089:
take pending approvals back through operator review before rollout, and do not
rewrite them.

Roll out disabled-by-default to controlled accounts after agent and human live
acceptance. Disabling the API unmounts it and leaves the tables in place, and the shared
rows are still kept up to date. v1 clients lose access to read state and keep their unsent intents until it
returns. Neither setting changes NIP-RS state or how NIP-RS requests are
processed. While enabled, v1 requests count against the signer's existing
API-call quota and share the existing writer database pool with other relay
work.
