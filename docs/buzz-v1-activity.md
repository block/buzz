# Private activity and read-state accessory API

`BUZZ_V1_ENABLED=true` opts into `/buzz/v1`; it is disabled by default.
Conversation history, live events, edits and deletion remain Nostr-authoritative.
This API neither replaces Nostr reads nor writes artificial signed events.
Legacy NIP-RS continues unchanged and is never read by the relay. A client
carries its NIP-RS read state over once with `import_through` (see
[Moving over from NIP-RS](#moving-over-from-nip-rs)).

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

`GET /buzz/v1/me/activity` returns one person's conversations and threads as
rows of one shape, newest first. It
is what a sidebar, an inbox and a threads view read. Mentions are a feed of
their own, `GET /buzz/v1/me/mentions` (below). A row carries only what
those views draw; filter and re-sort the rows on the client.

| Parameter | Meaning |
|---|---|
| `kinds` | Comma-separated: any of `channel`, `dm`, `solo`, `thread`. Default: all four. |
| `limit` | Rows per page, 1–500, default 100. A view that shows the whole list asks for 500. |
| `before` | The `next` cursor of the previous page. Valid only with the `kinds` it came from. |
| `since` | The `marker` of an earlier read: only what changed. Not combinable with `before` or `limit`. |

```json
{"imported":false,
 "rows":[{"kind":"thread","id":"<root id>","channel_id":"<uuid>","root_id":"<root id>",
   "at":1731550000,
   "preview":{"id":"<64-hex>","pubkey":"<64-hex>","created_at":1731549990,
     "text":"first 256 characters"},
   "unread":true,"state":"default"}],
 "removed":[{"kind":"channel","id":"<uuid>"}],
 "totals":{"channel":2,"dm":0,"solo":0,"thread":1,"mention":1},
 "next":"<cursor>|null","marker":"<opaque>|null","reset":false}
```

### Rows

| `kind` | `id` | Listed when | `preview` |
|---|---|---|---|
| `channel`, `dm`, `solo` | channel UUID | you are a current member of the channel. `dm` is a DM channel, `solo` a channel labelled `buzz.solo`, `channel` the rest | newest top-level message |
| `thread` | root event id | the thread has a reply, its channel is one you are in, and you are part of it: you replied, someone replied to your message, a reply tagged you, within the last 30 days; or you muted it or flagged it unread | newest reply |
| `mention` | the mentioning event id | a message of an eligible kind in a channel you are in tags you with `p`, arrived within `retention_seconds`, is not deleted and is not yours | the message itself |

`root_id` is present on thread rows and on mentions inside a thread. Names,
types, membership lists and archived state come from Nostr channel metadata,
not this API.

- **`preview`** is what to show. For a conversation it follows the channel
  window's rule: a message with no parent, or a depth-1 reply broadcast to the
  channel. A message tagged `["audience","agents"]` is never a conversation's
  preview; a thread's preview is its newest reply, whatever its audience. The
  latest edit of a conversation's preview is applied to its text; a thread's
  preview is the reply as stored. `preview.created_at` is the author's time,
  for display only. A deleted message is never returned: the row falls back to
  the previous one, or `preview` is null.
- **`at`** is the sort key: the relay's arrival time, in Unix seconds, of the
  message in `preview`. A conversation with no such message carries its
  creation time. A reply inside a thread moves the thread's row, not its
  conversation's. To also move a conversation up when one of your threads in
  it gets a reply, take the later of its `at` and its thread rows' `at`;
  thread rows carry `channel_id`.
- **`unread`**
  - conversation: its preview is someone else's and arrived after your read
    position, or you flagged it unread. A reply that is not broadcast never
    makes a conversation unread.
  - thread: its newest reply is someone else's and arrived after your read
    position, and you were last part of it (posted, were replied to, were
    tagged) within 30 days. Or you
    flagged it unread.
  - mention: it arrived after the read position of the channel timeline or
    thread it is in, and was not read on its own (`mark_read`). Marking that
    scope read reads its mentions.
  - A `muted` conversation or thread is never unread. A mention row inside it
    is still unread and still counted: a mention gets through a mute.
- **`state`** is `default` or `muted`.

A row has no mention count. To mark a conversation or thread as holding an
unread mention, look at the unread mention rows: each carries `channel_id`, and
`root_id` when it is inside a thread.

Nothing is unread before your account starts. Your first request to this API
starts it; every read position is floored at that moment, so a scope you never
marked counts only what arrived afterwards.

- **`imported`** (every list read; null with `reset`; not on the mention
  feed) is whether the person
  finished carrying their NIP-RS read state over (`finish_import`). While it
  is false, a client that also speaks NIP-RS keeps showing its NIP-RS unreads
  and imports them; it does not show the relay's.

### Order and paging

Rows sort by `at` descending (compared at microsecond resolution),
then conversations before threads, then `id`. `next` is null on
the last page. A page is cut short of `limit` when it would exceed the response
size; follow `next`.

Pages are separate snapshots: a row that moves up while you page can be missed
or seen twice. Page to fill a view; use `since` to stay current.

### Mentions

`GET /buzz/v1/me/mentions` is the mention feed: rows of the list's shape with
`kind` `mention`, newest first by arrival.

| Parameter | Meaning |
|---|---|
| `limit` | Rows per page, 1–500, default 100. |
| `before` | The `next` cursor of the previous page. Valid only with the `unread` it came from. |
| `unread` | `true`: only unread mentions. |

```json
{"rows":[{"kind":"mention","id":"<event id>","channel_id":"<uuid>", ... }],
 "next":"<cursor>|null"}
```

The feed is read when a person opens it, and paged as they scroll. It has no
`marker`, no `totals` and no `since`. A mention is a message, and a
connected client already receives it as an event; whether it is unread changes
only when a read position moves, which the client that moved it knows and
other devices learn from the row push below. To refresh the feed, read its
first page again.

A page examines at most 2,000 of the person's mentions. With `unread=true`, or
when many of them cannot be shown (deleted, or in a channel the person left),
a page can come back short, or empty, with a `next`: follow it. `next` is null
only at the end of the feed.

### Totals

`totals` is the number of unread rows of each kind over the whole set,
whatever the filter and page. `totals.mention` is your unread mentions, and
stops at 99. It comes with a first page of the list (no `before`) and with a
`since` read that found a change, a newly arrived mention included. It is null otherwise: keep the numbers you have.

### Staying current

A first page and every `since` read return a `marker`. `GET
/me/activity?since=<marker>` returns the rows that changed since then, the rows
that left (`removed`), fresh `totals` when anything changed, and a new marker.
Replace rows by `(kind, id)`. A `since` read may repeat a row that did not
change (rows changed in the 30 seconds before the marker are sent again).

`since` covers:

- every change you made: marks, a mention read on its own (it returns that
  scope's row and fresh `totals`), manual unread, state changes, imports, and
  the read position your own post moved;
- a new preview: a top-level message in a conversation, a reply in a thread,
  or an edit of a conversation's preview;
- joining or leaving a channel. Joining returns the conversation's row however
  old its last message is, and with it your thread rows in that
  channel. The join itself is not pushed: read `since` after
  it succeeds. `removed` names a conversation you left; drop its thread and
  mention rows with it (they carry `channel_id`).
- a thread your own change took off the list (a state set back to `default`
  or a flag cleared on a thread you are not otherwise part of). `removed`
  names the thread.

It returns no mention rows. A mention that arrived since the marker brings
fresh `totals`, and nothing else: the feed is read on its own.

It does not report a deletion that changes only a count, or a row whose only
change is that it aged out. `reset: true` means the marker is older than 30
days or too much changed (more than 1,000 conversations and threads): discard
the list and read page one. `since` honours `kinds`.

A `since` read costs what the person's own list costs, not what the community
did meanwhile: it walks the person's memberships and thread rows, looks each
shared row up by its key, and then reads only the rows that changed.

A WebSocket connection admitted as you also receives your own changes as
they commit, from any of your devices:

```json
["BUZZ","activity",{"rows":[ ... ]}]
```

The rows are the ones the `POST` below returned. The push is best effort and
unordered relative to reads; treat it as a hint and let `since` correct it.
New messages are not pushed this way: they already arrive as events.

### What a client does

- **On start and on reconnect, read `since`.** Not per event, and not on a
  timer. On reconnect, first wait a random 0–10 seconds: when a relay restarts,
  every client reconnects in the same moment, and the delay spreads their
  reads. Live events still arrive during the wait.
- **While connected, apply new messages to the rows yourself**, from the
  events you already receive. A message becomes its conversation's preview and
  makes the row unread when it is top-level or a reply broadcast to the
  channel, is not yours, is not tagged `["audience","agents"]`, and the
  conversation is not muted. A reply that is not yours does the same for its
  thread's row, whatever its audience. The
  relay pushes only the changes you made (below).
- **After joining a channel, read `since`** to get its row.
- **Keep the list and its marker across restarts**, so a start is a `since`
  read, not a first page.
- **Read the mention feed when it is opened**, and its first page again when
  it is reopened after a reconnect. While connected, add a mention to the top
  from the event that carries it.
- **When you mark a row read, mark its mention rows read locally**: those in
  the same channel timeline, or the same thread, that arrived at or before the
  message you marked through. Do the same for the rows of a `BUZZ` push.
- **Mute through this API.** `state` is where a mute is read and `set_state`
  where it is written, on every device. A client that still keeps mutes in its
  own synced preferences moves them here once: one `set_state` per muted
  conversation (100 per request), then stops writing them there.

## Writes

`POST /buzz/v1/me/read-state` accepts 1–100 independent intents:

```json
{"intents":[
 {"type":"mark_through","target":{"channel_id":"<uuid>"},"message_id":"<64-hex>"},
 {"type":"mark_through","target":{"channel_id":"<uuid>","root_id":"<64-hex>"},"message_id":"<64-hex>"},
 {"type":"mark_read","target":{"channel_id":"<uuid>"},"message_id":"<64-hex>"},
 {"type":"mark_unread","target":{"channel_id":"<uuid>","root_id":"<64-hex>"}},
 {"type":"mark_unread","target":{"channel_id":"<uuid>"},"message_id":"<64-hex>"},
 {"type":"set_state","target":{"channel_id":"<uuid>"},"state":"muted"},
 {"type":"import_through","target":{"channel_id":"<uuid>"},"through":1731550000},
 {"type":"finish_import"}
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
- **`mark_read`** reads one message of the scope (an anchor as for
  `mark_through`) without moving the read position: a mention of you stops
  being unread on every device, and the rest of the scope stays as it was.
  This is what a client sends when a mention, or a reply that tags you, has
  been on screen long enough to count as read but the person has not reached
  the bottom. A message that does not tag you has nothing of its own to read;
  the intent is applied and changes nothing. To read a thread's replies up to
  one the person saw, use `mark_through` on the thread: its replies are read
  in order.
- **`mark_unread`** flags the scope unread until the next `mark_through`.
  With a `message_id` it is "mark unread from here": if the read position
  had passed that message, it moves back to the instant before the message's
  arrival; the scope is flagged unread; and mentions in the scope from that
  message on that were read on their own are unread again. It only moves
  back, so it never reads a message before the one named: a position that is
  already earlier, or was never set, stays where it is. It is the one intent
  that moves a position backward. Messages from before the account started
  stay read (the floor below); only the flag shows.
- **`set_state`** sets `default` or `muted`.
- **`import_through`** and **`finish_import`** carry NIP-RS read state over
  (next section).

Posting a message moves your own read position to it, in the thread and, for a
top-level or broadcast message, in the channel. No intent is needed. It does not
clear a manual unread; only `mark_through` does.

The response is `{"outcomes":[...],"rows":[...]}`. Each intent commits on its
own and returns `applied`, `blocked` (missing or forbidden, deliberately the
same) or `invalid`, in request order. An ambiguous timeout or storage failure
returns `{"status":"unknown","retryable":true}`; retry the same operands, never
substitute a newer message. Because `mark_unread` with a message moves a
position back, a retried `mark_through` or `import_through` that lands after it
reads that message again: retry before sending later intents for the scope,
not after. Earlier committed outcomes survive later failures.
`rows` holds the updated conversation and thread rows of the applied targets,
in the list's row shape; a target in a channel you have not joined, or a thread
the list would not show, has no row.
Mention rows in a marked scope are not returned: flip them locally or read
`since`. `rows` is omitted when the rows could not be read within the request's
deadline after the writes committed.

A read position is the relay arrival time of its anchor, never the author's
time, which the sender chooses. The order is the relay's: send the anchors the
user actually saw. `mark_through` and `import_through` take the greatest; only
`mark_unread` with a message moves a position back. Arrival is the accepting
relay process's clock: it is not commit order, processes can differ by their
clock skew, and messages with the identical stamp are read together.

## Moving over from NIP-RS

The relay cannot read NIP-RS: its blobs are encrypted to their owner. A
client that holds them carries them over, and nothing on the relay is trusted
for a person until it has.

**What the relay offers.**

- **`import_through`** takes a scope and a NIP-RS frontier, `through`, in
  Unix seconds on NIP-RS's own clock (the signed `created_at` of the messages
  it covers). The relay finds the scope's newest message signed at or before
  it (a channel's timeline message, or a thread's root or reply) and advances
  the read position through that message's arrival. It never moves a position
  back and never clears a manual unread, so it is safe to send again and from
  every device; the greatest frontier wins, as in NIP-RS. A frontier that does
  not advance the position, including one from before the account started,
  writes nothing, so a replay does not show up in other devices' `since`. A
  channel looks at its newest 512 events at or before the frontier, and a
  thread at its newest 512 replies; a scope with nothing that old is applied
  and unchanged.
- **`finish_import`** records, once, that the person's read state has been
  carried over. Every list read reports it as `imported`. A person with no
  NIP-RS state still needs it: until it is sent, a v1-only client shows no
  unread state.

**What a client does.**

1. On its first v1 start for a community, before showing the relay's unread
   state, decode its merged NIP-RS state (what it shows today) and send one
   `import_through` per channel frontier and per `thread:` frontier newer than
   the 30-day NIP-RS floor, 100 per request. Every client runs this in the
   same rollout window, so start it after a random delay and back off, with a
   cap, on `unknown` outcomes or failed requests.
2. When every `import_through` came back `applied` or `blocked` (retry
   `unknown` ones first), send `finish_import`, even if there was nothing to
   import, and trust the list from then on.
3. Until `imported` is true, keep showing NIP-RS unreads. A client that only
   speaks v1 shows no unread state while `imported` is false, rather than
   everything unread.
4. While devices that have not upgraded may still read with NIP-RS, keep
   reading NIP-RS on every v1 start and reconnect, and forward any frontier
   that advanced as `import_through`. Keep writing NIP-RS frontiers too, so
   those devices see what was read on the upgraded ones. Stop both once no
   supported client release reads NIP-RS.

**What a person sees.**

- Nothing changes on the switch: the same channels and threads are read.
- Unread state from before the account started is not carried over as
  unread. Every read position is floored at the start, so a mention that was
  still unread under NIP-RS when the person first used a v1 client is read on
  the relay. This is deliberate: the switch can make old messages read, never
  unread. (A client that wants to keep a channel bold can send `mark_unread`
  for it during the import.)
- Mentions and replies read by dwell, and messages marked unread one by one,
  were kept on one device under NIP-RS and are not carried over. From the
  switch on they are kept on the relay and agree on every device.
- A device that has not upgraded keeps its NIP-RS behaviour. What it reads
  reaches the relay the next time an upgraded device of the same person
  starts or reconnects (step 4). What upgraded devices read reaches it through
  their NIP-RS writes, except per-message reads and unreads, which NIP-RS
  never synced.

## Stored state

| Table | One row per | Holds |
|---|---|---|
| `channel_heads` | conversation | newest top-level message with its arrival, text and latest edit |
| `thread_heads` | thread with a reply | newest reply with its arrival and text |
| `personal_read_frontiers` | person and scope | read position, state, manual unread, when the person was last part of the thread, their change counter at their last write |
| `personal_read_accounts` | person | when unread tracking started, their change counter, when their NIP-RS state was carried over (`imported_at`) |
| `event_mentions` | person tagged in an event | now also the event's arrival and thread root, and when the person read it on its own (`read_at`) |

The event log is the truth. The two shared tables are caches of it that prove
their own freshness: each row records the newest stored event it accounts for
(`seen_at`), and a read asks the log, with one index probe per row, whether
anything newer exists. A row that is missing, behind, or names a deleted
message is rebuilt from history before it is returned. So a lost write is
found by the next read (the exceptions are under Limits), and history from
before these tables is filled once by an operator (Rollout, below) so that
first reads do not have to.

- **On a message.** The storing transaction only records in memory what it
  stored. After it commits, a background writer in the same process coalesces
  per channel and writes the shared rows about 25 ms later, in its own
  transaction. A busy channel costs one write per interval per relay process,
  and no message transaction holds or waits for a shared row. The same write
  moves the author's read position and marks the thread as one the parent's
  author and the people tagged are part of.
- **On shutdown.** Once a relay has stopped serving it waits up to three
  seconds for the writes still in memory, so a deploy or a restart loses none.
  Only a process that is killed, or crashes, loses them.
- **On a read.** A first page and a `since` read check the person's joined
  channels and live threads and rebuild what is behind. A read rebuilds for at
  most 2.5 seconds, 64 rows per transaction, and four reads rebuild at once
  per process. Rows it did not reach, and the rows of a read that could not
  get a turn, are served as stored, and the next read continues. A
  conversation not yet rebuilt is still listed, with no preview and placed by
  its creation time; it is not shown as unread for messages older than the
  account's start, which is all a rebuild could add.
- **On an intent.** The person's account row is locked and its counter
  incremented; the scope's row is stamped with it. Counter order is therefore
  commit order, which is what makes the personal half of `since` exact.

The marker is `{counter, database time}`. A `since` read returns the person's
rows with a higher counter, plus shared rows and memberships changed
after the marker's time less 30 seconds.

Three counters say how often the accepted losses below happen. They should
stay at zero in normal running:

| Counter | Counts |
|---|---|
| `buzz_db_activity_write_failures_total` | shared-row writes given up on: a bound was hit or eight attempts failed. The next read rebuilds the row, except for an edit or a thread beyond the per-channel bound of one write interval (32 edits, 256 threads), which is rebuilt only when the row is next rebuilt for another reason. |
| `buzz_db_activity_people_writes_dropped_total` | writes of people's own rows given up on. Nothing rebuilds these (first limit below). |
| `buzz_db_activity_unwritten_at_shutdown_total` | channels still unwritten when the shutdown wait ran out. |

`buzz_db_activity_repairs_skipped_total` and
`buzz_db_activity_repairs_unfinished_total` count reads that could not get a
turn to rebuild, or ran out of time; both are expected while a deployment's
rows are first being built.

### Limits

These are accepted. Each is rare, and the counters above watch the first two.

- **A lost write of a person's own row is not repaired.** People's own rows
  are written separately from the shared rows and before them, and nothing
  rebuilds them. If that write is lost (the process was killed before it ran,
  or eight attempts failed), the author may see their own message as unread,
  and a person replied to or tagged in it is not added to its thread: the
  thread is missing from their list until its next reply. The mention row is
  unaffected.
- **A lost write of a shared row is repaired by the next read, with a
  hole.** After eight failed attempts the rows are marked unverified and
  changed, so the next read of any kind rebuilds them. After a killed process
  the next read still finds the gap, unless another event in that channel was
  written first (a reaction, a reply inside a thread, a message for agents
  only) or the lost message carries an author time older than one already seen
  (a slow sender clock, up to 15 minutes). Then the row is stale until the
  channel's next top-level message, and right again from it on.
- A message with an author time ahead of the relay's clock is checked for only
  once that time has passed.
- Threads you neither muted nor flagged leave the list 30 days after
  you were last part of them.
- Mention rows written before this migration have no arrival time and are not
  listed or counted until the backfill below fills them, for the last 30 days.
- The unread mention total, and each page of the mention feed, examines at
  most 2,000 of the person's mentions.
- The rebuild examines a conversation's newest 512 events and a thread's newest
  512 replies. A conversation whose newest 512 events hold no top-level message
  (a channel where everything since is reactions, edits or thread replies)
  shows no preview until a new one arrives.
- A `since` read walks all of the person's thread rows, live or not. They
  grow with every thread a person has ever been part of.
- At most 64 people tagged by one message, and 1,024 people per channel per
  write interval, are marked part of a thread.
- A thread stops being unread 30 days after you were last part of
  it, without a `since` change.
- 64 KiB write body; 1 MiB response. DB statement and lock deadlines and an
  eight-second request deadline bound the work.

Apply migrations 0105, 0106 and 0107 (or the equivalent desired schema).
0107 adds two nullable columns, `event_mentions.read_at` and
`personal_read_accounts.imported_at`, with no default and no index. 0105 creates
`channel_heads` and `thread_heads` empty, adds nullable or defaulted columns to
the three existing tables and makes `personal_read_frontiers.through_timestamp`
nullable. The two shared tables have no index beyond their primary key: every
read reaches them by key from the person's own rows. 0106 builds the one new index, on
`event_mentions` by person and arrival, inline and verifies it.
`event_mentions` is large: a deployment that applies migrations out of band
should apply 0105, build the index with `CREATE INDEX CONCURRENTLY` (the
statement is in 0106's header), then apply 0106, which skips an index that
already exists. The previous release never reads or writes the new columns, and its
inserts keep working against the migrated schema.

### Rollout

History stored before the release that writes these rows is filled once, by an
operator, after every relay runs that release. Best before clients read the
list, though not required: a client that already holds a marker gets the
threads the backfill adds from its next `since` read, and the mentions when it
next reads the feed.

1. `psql -X -v ON_ERROR_STOP=1 -f scripts/maintenance/backfill-activity.sql`.
   For messages of the last 30 days it fills the arrival time and thread root
   of mention rows, and makes people part of the threads they replied in, were
   replied to in or were tagged in. It works in windows of at most one hour
   and 5,000 messages that each commit, respects community deletion, never
   moves a read position or an engagement time backward, and ends with a
   validation query that returns `0|0`. Stop and rerun it freely.
2. `buzz-admin warm-activity`, once per community. It builds every
   conversation's row and the row of every thread with a reply in the last 30
   days, in the batches a read uses, and rebuilds only rows that are missing or
   behind. It is safe while the relay serves and safe to run again. A batch
   that keeps failing is skipped and reported, with a non-zero exit; run it
   again, or leave those rows to the first read.

Skipping either is safe: mentions and threads from before the release stay off
the lists, and first reads rebuild the shared rows themselves, 2.5 seconds at a
time.

## Compatibility

Clients must ignore unknown response fields and unknown row kinds. Requests
are strict: unknown parameters and intent types are rejected, so send a new one
only after the relay advertises a version that has it. The WebSocket frame is
additive: a client that does not know `BUZZ` frames ignores them.

The intents `mark_read`, `import_through` and `finish_import`, `mark_unread`
with a `message_id`, and the `imported` field were added to version 2 before
any client shipped it.

Not in this version: a window of rows around a given message, search cursors,
reading NIP-RS on the relay, per-row mention counts, reply counts, the message
a scope was last read through, following a thread you are not part of, and
a server-side unread filter for
conversations and threads (the client holds that whole list and filters it).
Each was left out because no view draws it yet; add it when one does.

## Open questions

- **Older clients.** How long must a mute set through this API also be written
  to the place older clients read it from?

## Privacy and lifecycle

These typed relational rows are signer-private application state, **not
self-encrypted**. Database operators can see reading progress; ordinary Nostr
queries, search and moderator interfaces do not expose it. No public receipts
are emitted. Storage grows by touched contexts, not observed messages, and has
no fixed context-count ceiling.

Leaving/rejoining does not erase progress; revoked access hides it. Soft-deleted
channels are inaccessible. None of these tables has a foreign key (migration
0102 dropped the frontiers' two), so a hard-deleted channel's rows, or a
deleted account's frontiers, stay until community erasure and are never read,
because every read starts from current membership. Community erasure
inventories these tables, `channel_heads` and `thread_heads` included. They
are written only through the relay's community-admitted transactions and carry
no write-fence trigger. There
is no new public account export/reset endpoint. Operator-assisted erasure/export
must use the established authenticated operational process and explicitly scope
both community and actor; never equate the read-time horizon with data erasure.

There is no down migration, and disabling the API is not a rollback. A relay
built before 0105 that restarts with `BUZZ_AUTO_MIGRATE=true` (the Helm default)
refuses to start on the migrated schema. Whole-community deletion run from a
build before 0105 rejects the two new tables. A deletion approved on the
earlier schema and not yet fenced fails structural revalidation after 0105:
take pending approvals back through operator review before rollout, and do not
rewrite them.

Roll out disabled-by-default to controlled accounts after agent and human live
acceptance. Disabling the API unmounts it and leaves the tables in place, and the shared
rows are still kept up to date. v1 clients lose access to read state and keep their unsent intents until it
returns. Neither setting changes NIP-RS state or how NIP-RS requests are
processed. While enabled, v1 requests count against the signer's existing
API-call quota and share the existing writer database pool with other relay
work.
