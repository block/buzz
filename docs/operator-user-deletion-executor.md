# Run and recover a user-deletion workflow

The workflow orders account gating, media cleanup, and per-community content
cleanup. Real requests stop at the unimplemented account gate; they cannot
complete through placeholder handlers. Demo requests simulate all stages and
never delete media or content.

Direct `media-batches` and `content-batches` commands remain available for
operator-controlled cleanup. Neither command installs or enforces the account
gate. Protection against concurrent writes by the target or their agents remains
unimplemented; neither command proves full account erasure. Use a disposable
database and versioned storage bucket when testing. Owned-community workflow
execution is unsupported.

## Content batches

Run deletion offline with the relay, workflow engine and external writers stopped.
Read-only preview does not require stopping them. This command does not install
a lasting account fence.

`content-batches` selects the user plus agents owned by that user **in the target
community**. Ownership in another community does not authorize cleanup here.
The start record reports excluded agent keys with missing or conflicting local
ownership. Every batch rechecks ownership; a change aborts the batch. Former
membership does not prevent cleanup of historical content.

With `--relay-pubkey`, content cleanup also selects private hidden-DM visibility
snapshots (kind 30622) for the user and currently owned agents. A snapshot must be
signed by that trusted relay, have no channel, and have exactly one canonical
`d` tag and one `p` tag matching its stored viewer coordinate. Conflicting,
malformed or untrusted snapshots are left outside the supported scope. Without
the trusted relay key, these snapshots are not selected. Selected snapshots and
their dependent delivery records are removed, including historical soft-deleted
rows. DM conversations, other participants' preferences and encrypted message
copies are not selected by this rule.

Content cleanup also removes signed file-metadata publications (kind 1063),
without deleting referenced blobs. Mentions and pending delivery rows are
removed before their source events.

Workflow cleanup deletes owned definitions with no runs, approvals or linked
schedule history, removing each definition and its unused schedule claims in the same transaction.
The complete group must fit `--batch-size` (maximum 100 rows); otherwise planning
fails before deletion. Increase the batch size for a larger group. More than 99
claims on one unused definition requires a separately reviewed cleanup.
Definitions with history and all run/approval rows are retained. For recorded
`event` causes, it clears the signer's saved trigger text and emoji; for recorded
`manual` causes, it empties the caller's input map. It clears the recorded
approver's note. These field changes also apply inside another owner's workflow.
Required trigger fields remain present, and every other field is preserved.
Retained runs, including waiting approvals, will read the cleared inputs if
resumed later. This change does not cancel those runs. The existing signed-event
cleanup of kind 30620 is unchanged; retained definitions here mean database rows.
Missing provenance, malformed input shapes, webhook payloads, execution traces
and errors are not selected. This is partial content cleanup, not a claim of
complete workflow erasure or stopped workflow execution.

Workflow rows use the same bounded preview, fingerprint and retry checks as
other content. New history or schedule claims block definition deletion before
any cascade can occur. No device installations or gateway delegations are deleted.

The handler preserves actual reply ancestry and other users' reactions by erasing
selected messages into structural tombstones. Plain links, quotes and unmarked
references do not require retaining their destinations. Other authors' signed
messages remain unchanged. Reply parent/root lookups use existing
`thread_metadata` indexes; no new permanent index or migration is required.

For replies without thread metadata, the handler probes the existing tags GIN
index for each selected message ID in lowercase and uppercase. It then uses the
relay's shared parser to distinguish actual replies from ordinary links. It does
not search for all messages marked as replies. Mixed-case references without
thread metadata may be missed; replies with thread metadata use binary IDs and
remain covered regardless of spelling.

This fallback runs during planning and before each batch that physically deletes
events. All probes share one SQL statement with a one-second timeout and a
10,000-match limit. A message matching several probes counts several times.
A timeout or overflow aborts cleanup; unusually many references to the selected
IDs may require metadata repair first. These bounds are safety limits, not
measured production latency guarantees.

Each batch installs transaction-local statement and lock timeouts before taking
the community lock. Its planned output is written before locking; the commit
receipt is written after commit. An output failure may leave earlier batches
committed; retry replans the remaining records. A lost database session also
loses the private worklist and causes execution to fail rather than continue
with incomplete checks.

## Code map: worker, policy, and persistence

The worker and user-specific routing live in `buzz-deletion`. PostgreSQL
transactions live in `buzz-db`. Both are libraries in the same CLI process;
there is no additional service or database-side workflow function.

| File | Responsibility |
| --- | --- |
| `buzz-deletion/src/user.rs` | Parse command arguments, call the user/shared stores or worker, print results |
| `buzz-deletion/src/user/worker.rs` | Claim, heartbeat, call transition, dispatch handler, repeat, release |
| `buzz-deletion/src/user/handlers.rs` | Dispatch real media/content or synthetic demo actions; save outcomes through the shared store |
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

The common store does not know the names `gate_user`, `purge_media`, `purge_content`, or
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
| `gate_user` | `purge_media` | Enter media only after saved gate success; real gate is not implemented |
| `purge_media` | Enter communities | After saved success, select the first uncovered community |
| `purge_content` | `purge_agents` or community complete | Real cleanup continues to agent cleanup; demos read saved `result.has_agents` |
| `purge_agents` | Community complete | Mark this community covered |

After a community completes, select the first uncovered community in inventory
order. After all are covered, finish the request. Missing `has_agents` is an
error, not an implicit skip. The inventory's version selects the currently
supported rules; future versions must retain the rules needed by active requests.
No full execution plan is created upfront.

If the real `purge_db` keeps any of the user's messages in a channel, it must
also keep the user's `channel_members` row for that channel, marked removed. The
relay refuses to open a private channel that has or had a member other than the
sender and the sender's own agents (kind:9002 in `NOSTR.md`), and it finds former
members only through these rows. Without the row, the channel could be opened and
its history, those messages included, made public.

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

Approval creates no stages. A real run requires `--delete`, then refuses the
unimplemented `gate_user` handler and leaves it pending. Repeating the run does
not fabricate progress. Use `--target demo:example` instead of `--pubkey` at
submission to exercise the full synthetic workflow without deletion.

The routing requires saved gate success before media, and saved media success
before community cleanup. A media failure keeps the same stage current; resume
after the saved retry deadline using the same request ID. A saved completed
media stage is transitioned without running media again.

`--next-step` executes one handler, transitions, and releases the lease. Without
it, an invocation executes up to 100 handlers. The worker prints an inspection
JSON document; media's file report goes to stderr. The direct command prints the
file report on stdout. Each file has its hash, filename when present,
keep/delete decision, versions, and outcome counts; uploader lists are omitted.

`--fail-stage purge_content`, `--stage-delay-ms`, and `--demo-no-agents-in` exercise
placeholder failures and branching only. Demo media success is synthetic; real requests reject these controls.
An injected community failure preserves the same retry semantics as before.
Ctrl-C releases the worker lease; after an abrupt kill wait for lease expiry.

## Shared agent discovery

Media, content, account, and profile cleanup use the same discovery utility in
`buzz-db/src/store/user_deletion/agent_ownership.rs`, including account scope
capture. It normalizes the human key, reads current direct ownership from
`users`, and retains `(community_id, agent_pubkey)` pairs. It does not infer
ownership from membership, recurse through agents, or exclude deactivated
profiles. A missing profile grants no agent cleanup authority.

Every invocation and retry discovers again. Content, account, and profile
handlers use the utility inside their existing batch transactions with row
locks when rechecking ownership. Profile cleanup compares against the identities
that remain after its own committed batches. Media discovers once per invocation
and uses that snapshot through inventory and deletion. Writers must remain
stopped throughout the operation.

Discovery preserves each caller's existing limits: media, content and profile
allow 10,000 distinct identities including the human; account discovery allows
10,000 agents. Account scope capture retains its existing total-key bound.
Media retains every owned community for each selected agent. Ownership profiles must
remain until all dependent cleanup completes; profile removal goes last.

Media has no separate cap on `(community_id, agent_pubkey)` pairs. One agent can
contribute a pair for every community where the human owns it, so the identity
limit does not bound the pair count. These pairs are retained in memory, including
the storage-matching hash set, and the full pair arrays are sent with each media
candidate-page query. They are outside the inventory budget described below;
memory use and query payload size grow with the number of owned agent profiles.

## Media discovery and remaining limits

Every invocation and retry fetches the human's current directly owned agents
from `users`, retaining each `(community_id, agent_pubkey)` pair. The human's
own uploads are global targets; an agent's uploads are targets only in communities
where that human currently owns it. A missing profile or a different owner in
another community does not authorize cleanup there. Ownership rows must remain
until media and other dependent handlers finish.
Candidate hashes come from `media_uploaders`, `media_uploaded` audit entries,
and storage upload claims. A bounded database query combines candidate discovery
with outside-uploader classification. Storage-only candidates receive the same
hash-restricted database check in batches. Current membership is not a safe
history filter: relay membership rows can be deleted on departure.

Sharing checks span all communities for the selected hashes. Any outside
uploader in either database source or an upload-record version protects the
file, including the same agent's uploads in an unowned community. No per-file
owner-list query is made. The report retains its deduplicated `target_pubkeys`;
community ownership is used internally for every media decision.

Storage discovery scans `_uploads/` for target claims, then reads candidate
claims in a second `_uploads/` listing to protect outside claims encountered
before a target claim. Blob and thumbnail versions are listed by `{hash}.`
prefix, never by an empty bucket prefix. A separate `_meta/` listing covers
sidecars, including ones whose community is absent from current membership or
upload claims. Unrelated Git objects and unrelated blobs are not listed.
The metadata namespace scans remain proportional to their namespace sizes.

All evidence and exact version IDs are collected before deletion begins.
Each listing is bounded to 100,000 pages of at most 1,000 entries. The disposable
in-memory inventory has a 128 MiB accounting budget and a 250,000-entry cap
across candidates, filenames, object versions, and listing markers. Accounting
includes inline data, string bytes, and a conservative 256-byte tree/allocator
allowance per entry. Individual fields and serialized version records are
limited to 16 KiB. The budget is not a process RSS limit: bounded listing/report
pages and database/HTTP clients allocate separately. It replaces the former
1 GiB disk inventory, so large jobs may reach capacity sooner. Exhausting a
limit aborts before mutation, with a capacity error; no partial inventory is
used for deletion. Retries rebuild discovery from PostgreSQL and storage.
Unversioned exclusive objects also abort. Upload claim objects are deleted
last, and the target's database uploader rows are removed only after that
file's storage processing succeeds, so a partial failure can be retried.

Shared files retain storage claims and audit history, even after the target's
uploader rows are removed. Those historical claims can protect a blob after
both uploaders have left; claim retirement is not implemented here. A bare blob
with no surviving attribution cannot be assigned to a user by this handler.
The result always reports `account_erasure_complete: false`.

## Index rollout

Migration 0092 adds uploader/hash and hash/uploader indexes to `media_uploaders`
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
