# User-deletion fencing: event database writes

This PR checks existing restrictions in the two shared event insertion functions in `buzz-db/src/store/event.rs`: `insert_event_on` and `insert_event_with_thread_metadata_tx`. Every event kind reaching those functions is checked. HTTP and WebSocket event handlers report the restriction as a client refusal.

## Transaction flow

1. Enter the existing community-admitted database transaction.
2. Read the event author's stored owner in this community, if present.
3. Acquire shared global author/owner identity locks and the scoped author lock in canonical order.
4. Read the owner again. If it changed while acquiring locks, reject for retry in a new transaction.
5. Read the author's global and community-scoped restrictions. Reject a restricted author.
6. Persist the event and its transactional derived changes. Commit or rollback releases the locks.

Human restrictions are global by public key; agent restrictions are community-scoped. The author's own restriction decides whether the event is rejected. An owner's restriction is not an additional authorization check for an independently admitted agent. Restrictions still apply when profile rows are absent or recreated.

The writer uses the existing identity lock keys used by restriction insertion. If an event transaction wins first, the fence waits for it to finish. If the fence commits first, a waiting event sees the restriction and is rejected. If the fence rolls back, the event may proceed. No user-specific lease is involved.

## Scope

The guard covers persistence through these two functions, including messages, replies, reactions, edits, votes, canvas, channel changes, deletion events and artifacts. Tests exercise actual callers and check that rejection rolls back transactional domain rows and reply counters.

Separate replaceable-event writers (profiles, emoji collections, bookmarks and workflow definitions), personal read state, uploads, git, audio and ephemeral activity are outside this PR. Authentication, agent creation/owner assignment, affected-user checks and background work are also separate work. This is not complete user-activity fencing.

There are no schema changes, lease changes, or changes to restriction insertion behavior. The existing restriction reader is exposed within the database module so event writers can read after acquiring the full ordered lock set. Tests install restrictions through the previously merged helper solely as fixture setup.

The real user-deletion worker remains disabled. Before enabling it, ownership/admission writes and other serving paths must be coordinated, and the worker must atomically fence the user and the approved owned-agent set. The owner recheck here only detects changes observed while obtaining locks; it does not implement a complete ownership-assignment protocol.

## Validation

Regression tests cover global/scoped restrictions, unaffected users and communities, missing profiles, fence commit/rollback, writes already in flight, observed ownership changes, and the channel-TTL lock interleaving. Relay tests exercise signed HTTP requests and WebSocket EVENT handling before and after restrictions, including replies and unchanged database snapshots on refusal.

With isolated PostgreSQL and Redis configured:

```sh
scripts/postgres-test-run.sh -p buzz-db -p buzz-relay --lib \
  -E 'test(/user_deletion_write_guard|user_deletion_message/)' --test-threads 4
```

The earlier 510-case live matrix validated a broader revision that included leases. It is not evidence of this narrowed revision's scope or coverage. Current validation results belong in the PR description.
