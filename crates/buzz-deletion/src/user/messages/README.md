# Standalone message deletion

`buzz_deletion::user::messages::handle` processes one database batch for a
`purge_messages` stage. It is intentionally absent from the CLI, dummy dispatch,
transition rules and background executor. Adding this module does not enable
real deletion in any existing workflow.

The caller supplies a claimed `EntityLease`, the current community stage, the
trusted relay public key from configuration, and a batch size from 1 to 100.
Call again while `MessageBatch.exhausted` is false. The handler selects remaining
rows, ordered by thread, rather than saving a cursor. It never changes community,
chooses the next stage, releases ownership or starts a heartbeat. The caller owns
those operations. A successful final empty scan marks only this stage complete.
Failures are recorded through the existing stage retry mechanism and propagated.

## Preconditions

The approved request must target a real hex key or npub and include this
participant/former-participant community. Stage inputs must match that exact
inventory. A global user fence from this request must exist. User admission,
workflow execution and external delivery must already be gated and drained.
That includes writes targeting the removed messages, such as other people's
reactions/edits; a pre-transaction target lookup must not later recreate a
dependent row after the batch commits.
**The current fence table alone does not enforce those serving paths.** Do not
invoke this handler against serving production until that integration exists.

Each batch briefly takes the existing community deletion lock exclusively,
then the request lock, then the user identity lock. This waits for admitted
database writes, prevents counter races during the transaction, and does not
change the community lifecycle. It temporarily blocks other writes to this
community. Lock waits are limited to two seconds and statements to ten seconds.
Active external write leases, claimed deliveries and nonterminal affected
workflow runs prevent deletion. These checks do not recall already delivered
messages or replace application-level drain.

The caller must renew its lease before a batch and leave enough time for the
entire transaction. The ten-second statement timeout is not a whole-batch
deadline. The request-row lock blocks heartbeat renewal during a batch; a
heartbeat waiting on that lock cannot revive a lease that has expired. Expiry
rolls back the batch. If the lease has actually expired, the old worker also
cannot save a failure: the current running stage remains durable and a new
worker can reclaim it with a higher generation. Reduce batch size if repeated
expiry prevents progress; this handler does not impose a new lease duration.

## What one batch removes

- Authored stream messages (9, 40001, 40002), forum posts/comments (45001,
  45003), reactions, forum votes, edits, individual pin/bookmark events,
  scheduled-message/reminder events (40006/40007), and diff events (40008).
  Old soft-deleted rows are included. Community-global private reminders,
  personal lists and other account state belong to later handlers.
- Relay-signed attributed messages only when the signer is the trusted relay
  key. Attribution follows the existing first-actor/first-valid-p rules. An
  arbitrary author's actor or mention tags never confer authorship.
- Dependent edits, votes and reactions on removed events, including reactions
  from other people. The first valid e tag selects an edit/vote target; the
  last selects a reaction target. Other people's messages, diffs, pins and lists
  are not removed merely because they reference a deleted message.
- Materialized reactions, event_mentions, push_match_queue, push_wake_outbox,
  operator_listener_outbox and delivery_log entries for the removed events.
- Terminal workflow runs whose trigger event or saved message_id matches a
  removed event, including saved text, traces and errors. Their approvals
  cascade. Workflow definitions remain. Scheduled-fire claims remain with
  their optional run pointer cleared, preventing duplicate scheduled execution.

Authored candidates are capped at 100; the complete dependent event set and
matched runs are capped at 2,000 each. Dependent traversal stops with an error
past 16 levels, never commits an incomplete traversal. A smaller batch can
resolve a fan-out limit; a single over-limit message requires further work.
Only IDs and flags are loaded, not message bodies or workflow traces.

All deletions and thread repairs commit together. Any SQL failure or lease
expiry rolls back the batch. Re-running after a successful commit cannot
subtract counts twice. A final bounded sweep removes orphan materialized
reactions authored by the subject even if their source events no longer exist.
It also removes terminal workflow copies identifiable by the saved trigger
author when their source event is already missing; active runs block completion.

## What remains

Thread metadata retains only ancestry needed by remaining events. It has no
message body, tags, author or signature. Counts are recomputed from visible
remaining messages using the server-arrival clock. Thread-window reads recognize
a missing root through trusted metadata and still return original signed
replies. No fake signed tombstone is manufactured. Posting new replies to a
physically missing parent and client refresh/notification behavior are separate
integration work; this PR tests retrieval of surviving replies.

Other people's messages, their mention tags and mention-index entries remain.
Moderation reports/actions, administrative records and the hash-chain audit log
remain. Other people's copied text and messages generated by their workflows
are not classified as the subject's messages just because the source was a
trigger. Profile, membership, personal-state, agent and storage cleanup remain
separate. This stage's result is not an assertion of complete account erasure.

## Files are an explicit blocker

Shared physical files are not safe to delete based solely on an author's
message. Until the media handler can prove which files remain in use, any
selected event or dependent edit/reaction with imeta, link-preview, URL/image/
thumbnail/hash/emoji tags or a content URL fails before deletion. NIP-30 custom
emoji tags carry image URLs even when the content contains only a shortcode.
This deliberately also blocks some ordinary links. The source events remain available for a retry
once file cleanup exists; the handler never drops file references and reports
success. This PR does not add a bypass or a pretend storage adapter.

## Local tests

Use the repository's isolated PostgreSQL lane:

```sh
scripts/postgres-test-run.sh -p buzz-deletion --lib \
  -E 'test(user::messages::postgres_tests)'
```

Tests exercise the real handler, signed events and the database. They cover
shared roots and replies, edits, both directions of reactions, mentions,
delivery records, workflow copies, failures/restart, tenant boundaries,
trusted-relay attribution, fences and stale leases. No production connection
or workflow integration is needed.

Separate safety cases cover saved-input mismatch, live external-write leases,
and expiry injected during count repair after admission and deletion. Each
asserts that source/dependent records survive and retry state is saved. A
replacement-worker case expires the lease between committed batches, reclaims
through the real API, rejects the old worker/heartbeat, and finishes with the
new generation. This does not simulate heartbeat contention inside a live batch.
