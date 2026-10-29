# Standalone owned-community and request-summary handlers

This slice adds callable Rust components and PostgreSQL-backed tests. It does
**not** enable owned communities in user intake, add a CLI command, dispatch the
new stage from the worker, or change transition rules. Existing user execution
still rejects owned-community inventories. The existing community CLI now calls
the shared submission function with the same operator behavior; approval and
worker execution remain unchanged.

## Components

| Component | Entry point | Responsibility |
| --- | --- | --- |
| Community submission | `buzz_deletion::community::submit_and_prepare` | Share the existing submit, inventory and freeze sequence between CLI and parent workflows |
| Owned-community handler | `buzz_deletion::user::owned_community::handle` | Check/submit the child and save its last observed state on the user stage |
| Summary handler | `buzz_deletion::user::summary::summarize_request` | Explain the request using a consistent snapshot of user workflow tables |
| Community persistence | Existing `DeletionStore::submit`; `deletion/lookup.rs` | One request-admission implementation, plus lookup by community ID |
| Waiting persistence | `buzz-db/src/store/entity_deletion/observations.rs` | Save an external observation under the parent lease/generation check |

`submit_and_prepare` is a regular Rust function, not an API object or network
endpoint. The community CLI and user handler both call it with the existing
`DeletionStore` and `MediaStorage`. It calls the existing `submit`, shared
`build_inventory`, and `freeze_inventory` operations. There is one request
insertion query; no separate user-specific community submission implementation.
It needs no Redis connection and requires the same operator authority as the CLI.
An approved inventory describes scope, not operator authentication or proof of
current community ownership.

The CLI supplies a host and no expected ID, retaining same-submitter preparation
retries and explicit replacement of aborted requests. The parent supplies its
approved community ID and no host. If both are provided, they must resolve to
the same community before insertion. The existing submission operation locks the
community row while checking and inserting. ID-pinned admission is create-only:
if a concurrent caller already submitted, the shared function reads and returns
that request unchanged, including aborted requests. SQL/transport errors still
propagate; they are not treated as submission conflicts.

## One owned-community handler invocation

The caller already holds the parent request lease and maintains its heartbeat.
It supplies the full `delete_owned_community` stage row. The handler reloads the
row through `start_stage`, validates its target/community against the approved
parent inventory, and requires `relationship: "owner"`.

1. **Lookup by community ID.** Call `DeletionStore::find_for_community`. If a request exists, read it without modifying
   its inventory, approval, provenance, or execution state.
2. **Submit only when missing.** Call the same `submit_and_prepare` function as
   the community CLI, pinning the approved community ID. Create an operator-origin
   request, prepare/freeze its inventory, and return its ID and current stage. Concurrent
   submissions converge on the existing community request. A child is never
   approved or executed by this handler.
3. **Persist the observation on the existing user stage.** If child logical
   deletion is complete (`retention_pending` with `completed_at`), save stage
   success. Otherwise return `OwnedCommunityOutcome::Waiting` and save
   `metadata_json.waiting`.

`waiting` contains the child ID, community ID/host, child stage, inventory
digest, last error/block reason, retry deadline, observation time, the reason
for waiting, operator instructions, and command argument arrays. Commands are
data for display; the handler never executes them.

| Last observed child state | User stage outcome | Operator action |
| --- | --- | --- |
| Submitted, operator-origin | Waiting | Finish preparation with the original submitter, then seek approval |
| Submitted, owner-origin | Waiting | Let the community drain prepare inventory and approve the recorded owner intent after validation |
| Inventoried | Waiting | Inspect the frozen inventory and separately approve the child |
| Approved or executing | Waiting | Let community drain run it, or use the existing community `run` command |
| Blocked | Waiting | Inspect and fix the cause, then use `deletions unblock` with operator identity and remediation reason |
| Aborted | Waiting | Inspect, then explicitly submit a replacement if deletion should continue; the next handler invocation follows it |
| Retention pending with completion time | Completed | Logical deletion is complete; retained-copy obligations remain visible in the saved result |

Waiting currently means `status = 'running'` plus `metadata_json.waiting`.
It does not increase retry counts, set an error, finish the row, or change
`is_current`. Success removes the waiting marker and saves `metadata_json.result`.
Neither outcome advances the parent workflow.

If preparation fails after child submission, the child row remains `submitted`.
The error is recorded on the user stage with a retry deadline and propagated.
On the next due invocation, lookup finds that same child and records a wait
with preparation instructions. An operator can resume preparation using the
existing `buzz-admin deletions submit --host ... --requested-by ...` command.
This initial implementation deliberately does not retry preparation during
lookup. A crash between creating the child and saving its ID is also
recoverable by community-ID lookup. No derived UUID or new linking table is needed.

The user stage tracks the approved community ID, not a particular child request.
If an operator aborts child A and submits child B for the same community, the next
handler invocation updates the same stage row with B's ID and observed status.
It neither changes nor approves B. The stage stays waiting until the current
child completes logical deletion. A previously observed child disappearing
without a replacement remains an error; the handler does not silently resubmit.
An old lease generation cannot overwrite the user's saved
observation or result. Failures to record a retry also propagate.

## Summarize the request using only our tables

`summarize_request(store, request_id)` reads `entity_deletion_requests`,
`entity_deletion_approvals`, and `entity_deletion_stage_history` through the
existing repeatable-read inspection. It accepts no community-store or media dependency.
It never calls lookup/submit, checks a child table, or changes workflow state.

The report includes:

- Parent target/status, approved inventory and approval evidence.
- Lease owner/generation, heartbeat, retry deadline/count/error, completion time.
- All reached stage rows, including completed results and retention notes.
- Every unfinished stage's ID, name, community, saved inputs, execution state,
  timestamps and error. A running row with an external observation is displayed
  as `waiting_external`; a failed row remains displayed as failed.
- For external waits: the saved child ID/status, observation time, reason,
  next operator action and suggested command arguments. These are explicitly
  **last observed**, not live community status.
- Inventory communities without any stage row yet. This is unreached work, not
  a claim that communities with rows have finished every possible stage.

For example, if A is awaiting approval, B has finished, and C has no stage yet,
the report lists A's saved approval instructions, B's completed result, and C
as not reached. If A completes independently, the summary still shows its saved
wait until A's **owned-community handler** runs again and records completion.
The summary never refreshes A itself.

`complete` requires the parent to be completed and all reached stages to be
completed. An empty unfinished-stage list alone is insufficient: later routing
may still need to create more stages. The summary also works for other external
waits recorded under `metadata_json.waiting`; it does not assume every wait is a
community deletion.

## What remains for workflow integration

The current transition code requires the current stage to complete before
advancing. Do not dispatch this handler from that loop yet: it would repeatedly
visit the unfinished row. A separate integration change must permit proceeding
past external waits, revisit those rows on a later run, and stop with a saved
unfinished request when only external dependencies remain. Summary reporting
must remain read-only. Claim/heartbeat/release stays the worker's responsibility.
Waiting currently clears `next_attempt_at`; integration must define an explicit
resume or a bounded revisit cadence before enabling automatic scheduling, so a
released waiting request cannot be claimed repeatedly in a hot loop.

## Test this slice

Configure a disposable local PostgreSQL administrator connection as described
in `crates/buzz-db/TESTING.md` (`BUZZ_POSTGRES_ADMIN_URL`, `PGHOST`, `PGPORT`,
`PGUSER`, `PGPASSWORD`; optionally `PG_BIN_DIR`), then run:

```bash
. ./bin/activate-hermit
./scripts/postgres-test-run.sh -p buzz-deletion --lib \
  -E 'test(user::owned_community::postgres_tests)'
```

The runner makes isolated databases per test. Tests use the real stores, shared submission function,
handler and summary. A loopback S3 fixture serves empty tenant-prefix listings;
no real object storage is accessed. Tests fixture child terminal state instead
of running destructive community deletion. They cover separate approval,
repeat lookup, stale summaries, child completion, preparation failure recovery,
concurrent parent/CLI admission, aborted requests, stale generations, input
mismatch, host/ID mismatch before insertion, CLI preparation retry, and propagation
of database failures even when a child already exists.
Replacement coverage saves A, aborts it, submits B through the shared CLI path,
then verifies the same user stage waits on B and completes only when B does.
Owned inventory is fixture-seeded because real user intake still rejects it.
