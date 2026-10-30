# User-deletion fencing: event database writes

The two shared event insertion functions in `crates/buzz-db/src/store/event.rs`,
`insert_event_on` and `insert_event_with_thread_metadata_tx`, refuse events
whose signer has a user-deletion restriction. Every event kind reaching these
functions is checked, including relay-signed events.

## Transaction flow

1. Enter a community-admitted, writer-pool READ COMMITTED transaction.
2. Read the signer's stored owner in this community, if present.
3. Acquire shared global signer/owner locks and the scoped signer lock. Each
   call sorts its global keys before its scoped keys.
4. Read the owner again. If it changed while acquiring locks, return
   `EventAuthorOwnershipChanged`; the caller must roll back and retry in a new
   transaction.
5. Query the signer's global and community-scoped restrictions together. If
   either exists, return `UserDeletionRestricted` and roll back.
6. Persist the event and its transactional derived changes. Hold the locks
   until commit or rollback.

Human restrictions apply across communities by public key. Agent restrictions
apply in their specified community. The signer's own restriction decides
whether the event is rejected; the owner's restriction is not an additional
check for an independently admitted agent. Missing or recreated profile rows do
not bypass restrictions.

If a writer holds the shared identity lock first, an exclusive fence waits for
its transaction to finish. If the fence commits first, the waiting writer sees
the restriction and rolls back. If the fence rolls back, the writer may proceed.
No user-specific lease is involved in this database-write protocol.

## Errors and atomicity

`event_write_error` maps `UserDeletionRestricted` to the client refusal
`restricted: identity is restricted for user deletion`. Ownership changes use
the retryable internal-error category: HTTP responds with 500; WebSocket
responds with `error: internal server error`. This classification permits a
retry; it does not implement client retries. Unrelated database errors retain
the caller's existing mapping rather than becoming restriction errors.

Archive/unarchive requests (9035/9036) validate consent without changing state.
Their persistence transaction stores the guarded request, changes
`archived_identities`, and stores the relay delta (8002/8003). A failure rolls
back all three. Notifications are dispatched after commit. Duplicate requests
do not reapply the archive change or create another delta.

The archival snapshot (13535) is refreshed after commit. A refresh failure
propagates to the caller; retrying the request refreshes the snapshot without
repeating the mutation. The committed request and delta remain available for
query/replay. This is not a durable background retry queue: a process exit
between commit and snapshot refresh can leave the snapshot stale until a later
refresh. Existing freshness/consent validation still applies to retries.

## Actual lock order and exclusive-worker requirements

Shared identity locking occurs at event insertion, not necessarily at the
start of the transaction. Artifact and channel writers can already hold domain
row/advisory locks. An artifact Move inserts the author's revision and then a
relay-signed removal, acquiring another global identity lock after the first
signer's scoped lock. Per-call sorting is not a transaction-wide lock order.

An exclusive fencing transaction must:

- Acquire community admission, then the deletion request row, then its complete
  global identity set, then its complete scoped identity set.
- Write only restriction/request/checkpoint state while holding those locks.
  Do not lock or mutate users, channels, artifacts, events or other serving
  domain rows. Read ownership without row locks and perform cleanup in later
  transactions.
- Never upgrade a shared identity lock to exclusive. Audit complete exclusive
  sets against transactions containing multiple signers before wiring a worker;
  per-call sorting alone does not prove arbitrary multi-key batches safe.

The domain-row restriction prevents a fence from waiting on a row held by a
writer that is itself waiting on the fence. PostgreSQL regressions exercise
channel-TTL and artifact Move interleavings and assert that no deadlock was
recorded. Those cases are evidence for the tested paths, not a proof about every
future worker or lock combination.

## Coverage and remaining work

Persistence through the two guarded functions includes messages, replies,
reactions, edits, votes, canvas, channel changes, deletion events, artifacts,
and identity archive/unarchive requests. A refusal rolls back domain changes
performed in the same transaction, including counters and notification events.

Separate replaceable-event writers (profiles, emoji collections, bookmarks and
workflow definitions), personal read state, uploads, git, audio and ephemeral
activity are outside this guard. Authentication, agent creation/owner
assignment, checks on affected users other than the signer, and background work
need separate coordination.

The real user-deletion worker remains disabled. Before enabling it, coordinate
all remaining serving paths and external work, and atomically fence the user
and the approved owned-agent set. The owner reread detects changes observed
while obtaining locks; it is not a complete ownership-assignment protocol.
Restriction insertion behavior and the schema are unchanged by event guards.

## Regression tests

The directly registered `user_deletion_*_postgres_tests` modules in `buzz-db`
and `buzz-relay` cover scopes, missing profiles, unaffected actors, commit and
rollback races, ownership changes, transport errors and transactional state.
The artifact fence race lives in `artifact_postgres_tests`.

With isolated PostgreSQL and Redis configured:

```sh
scripts/postgres-test-run.sh -p buzz-db -p buzz-relay --lib \
  -E 'test(/user_deletion/) or test(/artifact_move_and_identity/)' --test-threads 4
```
