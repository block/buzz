# Run and recover a user-deletion workflow

The workflow runs a placeholder account gate, **real media cleanup**, then
placeholder community actions. Completion does not mean the whole account has
been erased. Media cleanup requires `--delete` and writers to remain stopped;
the placeholder gate does not stop uploads for you. Use a disposable database
and versioned storage bucket when testing.

Submit with `--pubkey` to discover current memberships. Legacy demo intake
remains available for inspecting orchestration rows, but its synthetic target
cannot execute the real media handler. Owned-community execution is unsupported.

## Code map: worker, policy, and persistence

The worker and user-specific routing live in `buzz-deletion`. PostgreSQL
transactions live in `buzz-db`. Both are libraries in the same CLI process;
there is no additional service or database-side workflow function.

| File | Responsibility |
| --- | --- |
| `buzz-deletion/src/user.rs` | Parse command arguments, call the user/shared stores or worker, print results |
| `buzz-deletion/src/user/worker.rs` | Claim, heartbeat, call transition, dispatch handler, repeat, release |
| `buzz-deletion/src/user/handlers.rs` | Dispatch real media or placeholder actions; save outcomes through the shared store |
| `buzz-deletion/src/user/transition.rs` | `UserDeletionTransition.get_next_stage`: recover or choose the next saved position |
| `buzz-deletion/src/user/workflow.rs` | Allowed user stages, result-based branching, community-progress reconstruction |
| `buzz-db/src/store/entity_deletion/mod.rs` | Shared store definition and exports; no stage policy |
| `buzz-db/src/store/entity_deletion/requests.rs` | Read/list/inspect requests and calculate/verify inventory fingerprints |
| `buzz-db/src/store/entity_deletion/approvals.rs` | Persist approval evidence and parent status together |
| `buzz-db/src/store/entity_deletion/leases.rs` | Claim, heartbeat, release, and owner/generation checks |
| `buzz-db/src/store/entity_deletion/stages.rs` | Save handler outcomes and commit stage transitions atomically |
| `buzz-db/src/store/user_deletion/mod.rs` | User-specific store definition and exports |
| `buzz-db/src/store/user_deletion/requests.rs` | Submit user requests and validate user inventory during approval |
| `buzz-db/src/store/user_deletion/inventory.rs` | Validate approved inventory and construct handler inputs |

The common store does not know the names `gate_user`, `purge_media`, `purge_db`, or
`purge_agents`. `UserDeletionTransition` supplies a synchronous decision function
to `EntityDeletionStore.transition_stage`. The store locks the parent and loads
history, invokes that function while still holding the lock, checks the returned
operation against the current lifecycle, applies it, and commits. The decision
function performs no SQL or external work. Returning an error rolls back the
transaction. This avoids a gap between reading state and saving a decision.

The operations are initialize, resume, advance, and already finished. An advance
includes the expected completed stage ID, the decision to retain in history,
and the successor's name, community, and inputs (or no successor for completion).
The store enforces ownership and current-stage preconditions; the user policy
ensures that the chosen destination follows the user workflow rules.

User submission and inventory interpretation are deliberately separate from
common workflow storage, even though they use the same tables. User approval
runs its inventory validator inside the common approval transaction. Future
real PostgreSQL deletion operations belong in the user-specific database folder;
only the media handler currently performs real deletion. Existing community deletion
continues using its existing implementation.

## The execution contract

```text
approve: save approval, create no stages
run: claim parent request
    get_next_stage(lease, None) → full stage row
    handler(lease, row) → persist status and result
    get_next_stage(lease, Some(row.id)) → full next row, Finished, or WaitUntil
    repeat; release ownership on return
```

The worker owns claiming, heartbeats, shutdown, dispatch, and a bounded execution
budget. It does not inspect handler results, reconstruct community progress, or
choose branches. `get_next_stage` owns all those decisions. Specific rules live
in `user/workflow.rs`; the database transition transaction contains no
stage-name comparisons or two-community requirement.

The handler gets the whole row. `metadata_json.inputs` holds the target and
community input. The handler marks the row running and either saves success
under `metadata_json.result` or records a failure and retry deadline. Returning
success from the handler means its outcome was saved, including a retryable
failure. Failure to save propagates out of the worker.

### Durable state

| Record | State |
| --- | --- |
| `entity_deletion_requests` | Inventory/digest, overall lifecycle, lease owner/generation/expiry, heartbeat, retry deadline/error/count, completion time |
| `entity_deletion_approvals` | Operator, time, approved inventory digest; retained separately |
| `entity_deletion_stage_history` | Stage/community, current position, execution status, timestamps, inputs, result, transition decision, predecessor ID |

`is_current` means the workflow's durable position. It does not mean a handler
is presently running. In particular, **completed + current is valid**: the
handler finished, but the transition still needs to be committed.

The handler's success transaction sets `completed`, saves its result and finish
time, and leaves `is_current = true`. The next transition transaction checks the
lease, reloads that result, chooses an allowed branch, clears the old current
flag, saves its decision, and inserts the successor with its inputs. If there
is no successor, it records request completion instead. Those transition writes
commit together; an insertion failure leaves the completed old row current.

Every inserted stage stores `metadata_json.previous_stage_id` (null for the
initial stage). Reconstruction follows this chain, not timestamps or UUID sort
order. It checks inputs against the approved inventory, validates saved
transition decisions, and builds the set of completed communities. A persisted
`community_complete` decision covers a community even when a runtime branch
skipped a stage. Unknown stages, disconnected history, and missing results fail
without inventing progress. Reads are bounded to 10,000 reached stages and
inventories to 1,000 communities; these are resource limits, not a demo shape.

### Starting, branching, and finishing

Claiming saves ownership while preserving the lifecycle status. Thus a crash
between claim and initialization leaves an `approved` request with no stages.
The first `get_next_stage` creates the configured initial stage and sets the
parent to `running` in one transaction. A previously started request with no
current row and no terminal outcome is an error, not permission to create a
second gate.

The version-1 workflow rules are:

| Stage | Allowed outcome | Runtime choice |
| --- | --- | --- |
| `gate_user` | `purge_media` | Always enter request-wide media cleanup |
| `purge_media` | Enter communities | After saved success, select the first uncovered community |
| `purge_db` | `purge_agents` or community complete | Read saved boolean `result.has_agents` |
| `purge_agents` | Community complete | Mark this community covered |

After a community completes, select the first uncovered community in inventory
order. After all are covered, finish the request. Missing `has_agents` is an
error, not an implicit skip. The inventory's version selects the currently
supported rules; future versions must retain the rules needed by active requests.
No full execution plan is created upfront.

`previous_stage_id` is bookkeeping for uninterrupted execution. It must belong
to this request. After restart pass `None`; the database current row is the
source of truth. An old non-current stage is never transitioned twice.

| Restart point | Recovery |
| --- | --- |
| Before the first transition | Initialize an approved request with empty history |
| During a handler | Resume the unfinished current row after lease takeover |
| After a saved failure | Wait for the deadline, then retry the same row |
| After saved success, before transition | Process the completed current row's saved result; do not rerun its handler |
| After transition commit | Return the saved successor row |
| After terminal commit | A rerun returns success without changing saved state |

A lease lasts 60 seconds and handlers renew it every ten seconds. Handler and
renewal futures are polled independently in the same task, so a renewal waiting
for the handler's parent lock cannot stop the handler from committing. No
detached renewal task survives cancellation. Generation checks reject stale
workers' status, result, transition, heartbeat, and release writes.

This is one request at a time through an explicit CLI command. There is no relay
scheduler or automatic retry daemon. The media handler rescans on retry and treats absent versions as already removed;
external actions still need idempotence or reconciliation: a crash after an external action but before saving
its result may repeat that handler.

## Run against disposable test resources

Build `buzz-admin`, configure `DATABASE_URL` and the `BUZZ_S3_*` variables, and
apply the schema using the operator procedure in [database.md](database.md).
Use a real test public key and versioned test bucket. Stop every writer before
running with `--delete`; neither the dummy gate nor a database lease prevents
new uploads. Never point these examples at a live account for a smoke test.

The direct command uses the same media implementation without creating a workflow:

```bash
./target/debug/buzz-admin user-deletions media-batches --pubkey "$TEST_PUBKEY" --read-only
./target/debug/buzz-admin user-deletions media-batches --pubkey "$TEST_PUBKEY" --delete
```

To exercise the persisted workflow:

```bash
REQUEST_JSON=$(./target/debug/buzz-admin user-deletions submit \
  --pubkey "$TEST_PUBKEY" --requested-by operator-a)
REQUEST_ID=$(printf '%s\n' "$REQUEST_JSON" | jq -r '.id')
INVENTORY_DIGEST=$(printf '%s\n' "$REQUEST_JSON" | jq -r '.inventory_digest')
./target/debug/buzz-admin user-deletions approve "$REQUEST_ID" \
  --approved-by operator-b --inventory-digest "$INVENTORY_DIGEST"
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --next-step
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --next-step --delete
./target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
./target/debug/buzz-admin user-deletions run "$REQUEST_ID" --delete
```

Approval creates no stages. The first run completes the dummy gate and leaves
`purge_media` pending. The second run performs media cleanup and advances to
community placeholders (or finishes if there are no communities). Omitting
`--delete` at media saves a retryable failure without deleting anything. Media
failure keeps the same stage current; no community action runs before media
succeeds. Resume after the saved retry deadline using the same request ID.
A saved completed media stage is transitioned without running media again.

`--next-step` executes one handler, transitions, and releases the lease. Without
it, an invocation executes up to 100 handlers. Workflow stdout is one inspection
JSON document; media's file report goes to stderr. The direct command prints the
file report on stdout. Each file has its hash, filename when present,
keep/delete decision, versions, and outcome counts; uploader lists are omitted.

`--fail-stage purge_db`, `--stage-delay-ms`, and `--demo-no-agents-in` exercise
placeholder failures and branching only. They do not simulate media success.
An injected community failure preserves the same retry semantics as before.
Ctrl-C releases the worker lease; after an abrupt kill wait for lease expiry.

## Media discovery and remaining limits

The target set is the human and directly owned agents across communities.
Candidate hashes come from `media_uploaders`, `media_uploaded` audit entries,
and storage upload claims. A bounded database query combines candidate discovery
with outside-uploader classification. Storage-only candidates receive the same
hash-restricted database check in batches. Current membership is not a safe
history filter: relay membership rows can be deleted on departure.

Sharing checks span all communities for the selected hashes. Any outside
uploader in either database source or an upload-record version protects the
file. No per-file owner-list query is made.

Storage discovery scans `_uploads/` for target claims, then reads candidate
claims in a second `_uploads/` listing to protect outside claims encountered
before a target claim. Blob and thumbnail versions are listed by `{hash}.`
prefix, never by an empty bucket prefix. A separate `_meta/` listing covers
sidecars, including ones whose community is absent from current membership or
upload claims. Unrelated Git objects and unrelated blobs are not listed.
The metadata namespace scans remain proportional to their namespace sizes.

All evidence and exact version IDs are collected before deletion begins.
Each listing is bounded to 100,000 pages of at most 1,000 entries; temporary
SQLite inventory is bounded to 1 GiB. These are failure guards, not limits on
how many files a user may own. Exhausting either aborts before mutation.
Unversioned exclusive objects also abort. Upload claim objects are deleted
last, and the target's database uploader rows are removed only after that
file's storage processing succeeds, so a partial failure can be retried.

Shared files retain storage claims and audit history, even after the target's
uploader rows are removed. Those historical claims can protect a blob after
both uploaders have left; claim retirement is not implemented here. A bare blob
with no surviving attribution cannot be assigned to a user by this handler.
The result always reports `account_erasure_complete: false`.

## Index rollout

Migration 0089 adds uploader/hash and hash/uploader indexes to `media_uploaders`
and equivalent partial indexes for `media_uploaded` audit entries. Existing
community keys remain unchanged. Apply schema before deploying the cleanup code.
The migration builds missing indexes under a 1-second lock timeout and a
5-second statement timeout and verifies pre-existing definitions and validity.
It does not refuse populated tables merely because they contain rows.

Large deployments with auto-migration disabled can prebuild the exact indexes
outside a transaction, following [thread-window-deployment.md](thread-window-deployment.md):

```sql
CREATE INDEX CONCURRENTLY idx_media_uploaders_actor_hash
  ON public.media_uploaders (pubkey, sha256);
CREATE INDEX CONCURRENTLY idx_media_uploaders_hash_actor
  ON public.media_uploaders (sha256, pubkey);
CREATE INDEX CONCURRENTLY idx_audit_media_actor_hash
  ON public.audit_log (actor_pubkey, object_id) WHERE action = 'media_uploaded';
CREATE INDEX CONCURRENTLY idx_audit_media_hash_actor
  ON public.audit_log (object_id, actor_pubkey) WHERE action = 'media_uploaded';
```

Then apply the migration, which verifies the indexes without rebuilding them.
A failed concurrent build must be inspected and repaired before retrying.
These additive nonunique indexes remain compatible with old relay writes.
The normal sqlx migration-version rollback constraints still apply: recover
with a corrected binary retaining the migration history, rather than deleting
migration records or changing checksums. Workflow requests and history are
retained for inspection; this scaffold has no automatic retention job.
