# Scheduled messages

The relay owns a durable, private queue of exact user-signed kind:9 messages.
Delivery runs without an open client. The desktop/web composer prepares ordinary
messages, mentions, attachments and thread tags before asking the host signer to
sign with the selected future timestamp. No account key is stored in the queue.
The scheduled timestamp is the event's timestamp; after an outage a late delivery
appears at its originally selected time in event history.

## Enable and roll back

This is a two-phase rollout. The feature is off unless
`BUZZ_SCHEDULED_MESSAGES=on` (exact value). When off, the endpoint returns 404,
the worker does not run and NIP-11 omits `buzz-scheduled-messages-v1`.

1. Apply migration 0087, or the matching desired-state table and indexes. It only
   creates a table and indexes; it has no backfill, trigger, function or FK.
   Older relays remain compatible with this expanded schema.
2. Replace **every relay process sharing this writer**, with the feature still
   off, so every replica-fence probe understands pending queue reservations.
   Confirm that no older probe process remains, including workers and canaries.
3. Enable the flag, then deploy clients that recognize the advertised extension.
   Verify delivery with clients closed, queue access isolation and replica reads.

To pause delivery, turn the flag off on every worker. Existing queue rows remain;
when re-enabled they are rechecked and delivered late. Turning it off alone does
not cancel work already in flight. Never downgrade to reservation-unaware probes
while pending rows exist. Keep the reservation-aware binary deployed, or hold
replica routing closed and explicitly resolve all queued work before downgrading.

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

Mutations return `{"accepted":true}` after commit. Listing returns `messages`
with `id`, `revision`, `event`, `send_at`, `status`, `error`, plus
`sent_history_limit:100`. Active work is capped at 256 per owner/community; the
list includes all scheduled/failed rows and the last 100 sent rows. Cancelled
rows remain tombstones and are omitted. Queue state follows community deletion
inventory and purge. Sent rows and tombstones are retained until community purge;
the list limit is not a retention promise.

Identical operation retries succeed after an acknowledgement is lost. Conflicting
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

Transient storage/access failures retain work with exponential backoff. If a
failed insert has rolled back its claim, recording backoff reacquires only the
same pending revision; cancellation, edits and successful delivery are preserved.
Permanent refusals stay visible as failed; users may edit/reschedule or cancel.
Regular ingest side effects and fan-out remain shared. Like ordinary ingest,
external side effects after the database commit have their existing recovery
semantics; the queue guarantees one durable event, not exactly-once execution of
every external consumer.

## Replica completeness

Ordinary channel writes enforce a commit-time timestamp floor of 960 seconds.
A previously queued event may legitimately be older after downtime. Only a held
scheduled claim may widen its transaction-local floor, for its exact stored event.

Every reservation-aware writer probe caps its ordinary proof wall at
`min(send_at)-301` across pending rows, before committing the heartbeat token.
The event timestamp is at least `send_at-300`, so it is strictly above every
proof that did not yet observe its commit. The reservation disappears in the
same commit that stores the event; a later heartbeat therefore orders after the
event's commit. Cancellation removes the reservation without publishing a row.

Admission uses the **database clock after lock waits**: a new/revised reservation
must have `send_at >= DBnow-300`, and `created_at >= send_at-300`. Thus its earliest
possible timestamp, `DBnow-600`, stays newer than an already proved ordinary wall
(`S-965`). Transactions open during a probe are additionally bounded by its
oldest-open-transaction term. This preserves previously retained proofs even
when a reservation is created after their heartbeat. Host clock skew cannot
introduce a retroactive reservation. An overdue pending item may conservatively
hold the replica wall back until sent/cancelled; reads then use the writer.

The proof depends on every probe using reservations, which is why enabling during
a mixed-version rollout and downgrading with pending work are unsupported.

## Local verification

Use an isolated PostgreSQL/Redis relay with the feature enabled. From the modern
app checkout, run `BUZZ_SCHEDULED_TEST_RELAY=http://127.0.0.1:3000 pnpm exec vitest
run src/features/relay/scheduled-messages.live.test.ts`. The test refuses remote
hosts, uses fictional random keys, and exercises authenticated queue operations
through the real app service. The relay's database regressions additionally cover
overdue insertion, replica reservation release, lost claims, stale revisions,
worker exclusion, retry backoff and community deletion.
