# Buzz v1 channel heads

A sidebar that shows each channel's newest message and sorts by recent
activity needs two facts per channel. Reading them from the event log costs
one scan per channel on every app open. The relay instead keeps one stored
row per channel, so the read is a join over the caller's memberships plus one
index probe per channel that proves the row is current.

`BUZZ_V1_ENABLED=true` serves the route; NIP-11 then advertises
`buzz_v1.channel_heads: true`. A relay without it leaves clients on their
existing behaviour. The table is maintained whether or not the route is on.

## The row

`channel_heads` has one row per `(community_id, channel_id)`:

| Field | Meaning |
|---|---|
| `activity_at`, `activity_pubkey` | Newest message at any thread depth, and who posted it. Kinds 9, 40002, 40008, 45001, 45003. |
| `head_id`, `head_at`, `head_pubkey`, `head_text` | Newest top-level message: no thread metadata, depth 0, or a depth-1 reply broadcast to the channel. Kinds 9, 40002, 40008, 45001. A message tagged `["audience","agents"]` is activity but never the head. `head_text` is the first 256 characters. |
| `head_edit_at` | Set when `head_text` comes from a kind 40003 edit of the head. |
| `seen_at` | The newest stored event of the channel, of any kind, that the row accounts for. Null until a read has filled the row from history. |
| `updated_at` | Database time of the last change; drives `since`. |

Times are the author's signed `created_at`. Two messages in the same second
are ordered by lower event id, as in the channel window.

Rows are display state. They never grant access: every read joins current
membership and rechecks it before release. Unread state is not part of the
row.

## Keeping the row right

The event log is the truth. A row is a cache of it that can prove its own
freshness: a read asks, with one index probe per channel, whether any stored
event is newer than the row's `seen_at`. A row that is behind is rebuilt from
the log before it is returned. So no writer has to be reliable for a read to
be right, and the message path can stay free of the row.

- **Ingest.** A stored event is recorded on its write transaction. After
  that transaction commits, the result is handed to a background writer in
  the relay process. The writer keeps one pending entry per channel, merging
  by the same "newer wins" rule, and writes them every 25 ms in its own
  short transactions. A message never holds or waits for a channel's row, so
  a burst into one channel costs one row write per interval and messages do
  not queue behind each other. Ingest advances `seen_at` only on a row a
  read has already filled; a row it creates stays unverified.
- **Edit.** An edit whose target is the current head replaces `head_text`.
  Of two edits in one second the higher id wins, as in the timeline.
- **Fill.** A read that meets a row that is missing, unverified, behind the
  log, or whose head is deleted takes the row, then merges the channel's
  history into it: the newest 512 events of the channel are examined for the
  latest activity and the newest top-level message, and `seen_at` is set to
  the newest event read. This is also how rows first appear: there is no
  backfill. Filling is done in batches of 100 channels, each its own short
  transaction. When another reader already holds the rows, the read serves
  what is stored instead of waiting.

A failed row write is tried three times, merged with whatever arrived for
the channel in between. The writer waits at most 250 ms for a row a reader
is rebuilding. A row update that is still lost (the process exits inside the
25 ms, every attempt fails, more than 8,192 channels are pending, or the
event was stored by a relay version without this code) is repaired by the
next read of that channel's row. `buzz_db_channel_head_failures_total`
counts them.

`seen_at` never runs ahead of the database clock, so an event signed with a
fast clock leaves its row reading as behind until that time has passed,
rather than hiding what is stored meanwhile.

Rebuilding uses writer connections, which message writes need more. At most
four reads rebuild at once per relay process; a read that waits two seconds
for its turn, or whose rebuild fails, serves the rows as stored and the next
read tries again. `buzz_db_channel_head_repairs_skipped_total` counts those.
A row that has never been filled is served with no activity and no head.

Deletion has no hook. A read joins each row's head to its event; when the
head is deleted the read clears it, compare-and-set on the head id, and fills
the row again. A `since` read always includes a row that is behind or whose
head is deleted.

## Reading

`GET /buzz/v1/me/channel-heads` uses the same NIP-98 authorization, rate
limit, membership recheck, eight-second budget and 1 MiB response limit as
the other `/buzz/v1` routes.

```json
{
  "as_of": 1794600000,
  "truncated": false,
  "channels": [
    {
      "channel_id": "…",
      "activity_at": 1794599990,
      "activity_pubkey": "…",
      "head": {
        "id": "…", "pubkey": "…", "created_at": 1794599900,
        "text": "…", "edited": false
      }
    }
  ]
}
```

- No parameters: every joined, nondeleted channel, most recent activity
  first, at most 1,000. `truncated` means older rows were left out, either
  past 1,000 or to fit the response limit; ask for those by id.
- `?since=<as_of>`: only rows changed since that read. The relay widens the
  bound by 30 seconds to cover a slow commit, so a client must accept a row
  it already has.
- `?channel_ids=<uuid>,<uuid>`: 1–100 named channels, for a newly joined
  channel or a truncated list. A channel that is absent from the result is
  not a joined, nondeleted channel.

A client reads once at open and again with `since` after a reconnect. In
between, live channel messages move its copy of the rows forward by the same
rules.

## Limits

- The fill looks at the newest 512 events of a channel. A channel whose
  newest top-level message is older than that fills with no head until one
  is posted.
- `activity_at` is not recomputed when the newest message is deleted.
- A lost row update is noticed through `seen_at`. If a later event in the
  same channel is written normally before any read, `seen_at` moves past
  the lost one and the read no longer notices. The row then misses that
  message until the channel's next top-level message or fill. This needs a
  lost update followed, before the next read, by a reaction or thread reply.
- Two events in the same second share a `seen_at`, so a lost update for the
  second is noticed only once a later event arrives. The same holds for a
  lost update of an event signed earlier than the row's `seen_at`.
- An edit of the head that is written before the head itself, by another
  relay process inside the same 25 ms, is not applied.
- Deleting an edit does not restore the earlier text of the head.
- The row is written up to 25 ms after the message commits. A read inside
  that window normally still returns the message, because the row is behind
  and is rebuilt; only a message in the same second as the row's `seen_at`
  can be missing from a read for those 25 ms.

## Rollout

Rows are filled on first read and verified on every read, so relay versions
with and without this code can run side by side, and a rollback needs no
cleanup. While old processes are still storing events, the limit above
applies to what they store.
