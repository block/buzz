# Scheduled messages

The relay owns a durable, private queue of exact user-signed kind:9 messages.
Delivery runs without an open client. The desktop/web composer prepares ordinary
messages, mentions, attachments and thread tags before asking the host signer to
sign with the selected future timestamp. No account key is stored in the queue.
The scheduled timestamp is the event's timestamp. Delivery can retry for up to
15 minutes. If the window passes, the queue shows a missed-delivery reason and
the author can choose a new time.

## Enable and roll back

This is a two-phase rollout. The feature is off unless
`BUZZ_SCHEDULED_MESSAGES=on` (exact value). When off, the endpoint returns 404,
the worker does not run and NIP-11 omits `buzz-scheduled-messages-v1`.

1. Deploy the [deletion compatibility release](https://github.com/block/buzz/pull/9744)
   to every relay, deletion executor and operator CLI. This release establishes
   the rollback floor once the scheduling table exists.
2. Apply migration 0109, or the matching desired-state table and indexes. The
   migration creates a table and indexes with a five-second lock timeout and a
   30-second statement timeout.
3. Restart relay processes that receive NIP-FI disconnect commands after applying
   the migration. Check that all of them support durable cancellations before
   admitting federated schedules.
4. Enable the flag and verify delivery with clients closed, queue isolation,
   and issuer disconnects.

To pause delivery, turn the flag off on every worker. The original delivery
deadlines still apply. Drain or cancel federated schedules before downgrading
disconnect handlers to a version that predates durable cancellations.

## Protocol

`POST /scheduled-messages` uses the ordinary member HTTP admission path: NIP-98
with an exact URL/method/body hash, replay protection, community resolution,
NIP-FI policy, rate limits and current relay membership. Owners cannot specify a
different community or queue owner. The body is at most 40 KiB; signed events at
most 32 KiB and content at most 24 KiB. Only kind:9, one UUID `h` destination and
future timestamps within 120 days are accepted. The sender must currently hold a
non-guest conversation membership. Dispatch rechecks normal ingest permissions.

| Action | Body fields besides `action` |
| --- | --- |
| `list` | None |
| `create` | `id` (UUID idempotency key), `event` |
| `update` | `id`, `revision` (integer), `event` |
| `cancel` | `id`, `revision` |
| `send_now` | `id`, `revision`, freshly signed `event` |

Exact accepted create and mutation retries are reconciled by owner, queue ID, revision, and signed event before freshness checks. Send-now uses the signed timestamp for its queue time, and retries must reuse the same signed event. A changed event, cancelled row, or superseded revision is not an accepted replay.

Mutations return `{"accepted":true}` after commit. Listing returns `messages`
with `id`, `revision`, `event`, `send_at`, `status`, `error`, plus
`sent_history_limit:100`. Active work is capped at 256 per owner/community; the
list includes all scheduled/failed rows and the last 100 sent rows. Cancelled
rows remain tombstones and are omitted. The enabled relay prunes sent rows and
tombstones in small batches starting 30 days after the later of the scheduled
time and the transition to sent or cancelled. Community deletion purges all
queue rows.

Identical operation retries succeed while the matching queue row is retained. Conflicting
revisions or rebinding an event already attached to another queue ID return 409.
Send-now preserves the reviewed content and tags and replaces only its timestamp
and signature. Refresh the list before changing another revision. A response loss
is uncertain acceptance: retry the same create ID/event, never invent a second
message to recover it.

This HTTP resource holds relay execution state rather than a new public Nostr
event kind. The only delivered artifact is the exact ordinary signed event. Queue
contents are never broadcast or exposed in public event queries.

## Atomic delivery and recovery

Each one-second bounded sweep scans at most 100 due candidates, with randomized
startup phase. A worker admits one community transaction and claims one row with
`FOR UPDATE SKIP LOCKED`. Edits, cancellation and sending serialize on that row.
The shared ingest path performs ordinary signature, channel, ban, threading,
media and authorization checks. Event insertion, mentions, push work, thread
metadata and sent status commit on the held claim transaction. A lost claim
connection cannot later publish. Stable event IDs deduplicate crash recovery;
an already stored event, including a deleted event, is reconciled as sent.

A failing row does not abort the rest of the sweep. Transient storage and
access failures use exponential backoff within the delivery window. If a failed
insert released its claim, the relay records backoff only for the same pending
revision while it is still due. Missed deliveries and permanent refusals are
visible as failed rows with a reason. Users can reschedule or cancel them.
Regular ingest side effects and fan-out remain shared. Like ordinary ingest,
external side effects after the database commit have their existing recovery
semantics; the queue guarantees one durable event, not exactly-once execution of
every external consumer.

## Release window

Workers publish between the signed time and 900 seconds after it. They use
ordinary ingest and the existing database timestamp floor. The worker checks
the deadline again on the write transaction, before committing the message and
sent state. An already-stored message can still reconcile as sent after the
window. The existing replica proof applies to these writes.

## Federated identity

The queue records the issuer, verified assertion issuance, and agent owner
when a protected request admits a message. An authorized issuer disconnect
records the revocation and cancels matching pending rows across communities
in one transaction. Admission and disconnect serialize by issuer, and
dispatch and cancellation serialize on the queued row. If the transaction
fails, the disconnect returns 503 before closing sessions. The issuer must
retry with a freshly signed command.

A cutoff covers assertions issued at or before it. Other issuers and newer
assertions keep their schedules. Records remain effective until the later of
the command deadline or its recorded time, plus the configured maximum
connection lifetime. This uses the deferred-action rules proposed in
[NIP-SL and NIP-FI](https://github.com/block/buzz/pull/9912). The queue still
uses the HTTP protocol described above.

## Local verification

Use an isolated PostgreSQL/Redis relay with the feature enabled. From the modern
app checkout, run `BUZZ_SCHEDULED_TEST_RELAY=http://127.0.0.1:3000 pnpm exec vitest
run src/features/relay/scheduled-messages.live.test.ts`. The test refuses remote
hosts, uses fictional random keys, and exercises authenticated queue operations
through the real app service. The relay's database regressions additionally cover
bounded release, lost claims, stale revisions, worker exclusion, retry backoff,
issuer disconnects and community deletion.
