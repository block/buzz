# Rehearse a user-deletion workflow

A persisted rehearsal runs the current media, content, account and profile
planners without deleting user data. It exercises approvals, leases, retries,
stage transitions and terminal reporting. Gate and global cleanup are explicit
simulations. Completion means **inspection finished**, never **erasure finished**.

This is a development/operator command, not an online deletion implementation.
Use an isolated database and storage bucket. A rehearsal writes its own request,
approval and stage journal. For an owned community it also submits or adopts a
real community-deletion request. It never approves or executes that child.
Existing child requests keep their independent state; an already approved child
may be executing independently. Inspect child IDs in the report.

## Submit, approve, run

Build `buzz-admin` from this branch. Configure `DATABASE_URL` and the storage
settings required by `user-deletions media-batches` (`BUZZ_S3_*`). Storage must
support version listing. No Redis or running relay is needed by the worker.

```sh
buzz-admin user-deletions submit --pubkey <human-key> --read-only \
  --requested-by <operator> --run-id <new-uuid>
buzz-admin user-deletions inspect <request-id>
buzz-admin user-deletions approve <request-id> --approved-by <operator> \
  --inventory-digest <digest-from-inspect>
buzz-admin user-deletions run <request-id> --read-only
```

Omit `--run-id` to generate one, printed to stderr before submission. If the
response is lost, retry submission with that ID, the same subject, operator and
reason. It returns the existing request with its original inventory. A different
run ID starts a fresh rehearsal. `run` and `run --next-step` always use the
**request ID returned by submission**, not the run ID.

The request target is `rehearsal:<run-uuid>:<canonical-pubkey>`, preserving the
existing database uniqueness constraint without a migration. Approval binds its
mode, run ID and frozen community inventory. Legacy `rehearsal:<pubkey>` identities
remain readable; histories created by older draft workflow definitions may need
a fresh submission. Destructive requests retain their existing per-subject
uniqueness and approval. `--delete` refuses a rehearsal request, including one
that has completed.

## Saved stage sequence

1. `gate_user`: simulated, no admission fence installed.
2. `purge_media`: real upload inventory and storage-version preview, including
   currently owned agents in their communities.
3. For each community:
   - `delete_owned_community` if the approved relationship is owner: submit/adopt
     child intent, save its ID/status, continue without awaiting deletion.
   - Otherwise `purge_content` → `purge_account` → `purge_profile`, all real previews.
4. `global_cleanup`: simulated, no deployment-wide account records changed.
5. `remaining_cleanup`: print and save the report, then atomically finish the
   request through the existing transition function.

The last community automatically transitions to global cleanup. With no stored
communities, media transitions directly to global cleanup. Rehearsal discovery
inspects **every stored community**, matching `account-scope`, so former
memberships, residual records and agent-only scopes cannot disappear merely
because the human left. An unrelated empty community produces empty plans.
Unavailable or non-active communities and unresolvable ownership still fail
visibly; this milestone does not invent a disposition for those cases.

Profile previews report prerequisites still present in the database: earlier
previews proposed deleting content or memberships but left them intact. These
are observations, not proof that a destructive run could proceed. Actual profile
deletion and the standalone strict profile preview retain their existing checks.
Database, storage, ownership and planning failures remain failures.

Owned-child submission/preparation errors stop the stage. An existing blocked or
aborted child is recorded for operator intervention and does not count as a
successful handoff. A child already submitted before a crash is adopted without
duplicate submission. Its report may still say preparation or approval is needed.
Only the community deletion workflow can establish its eventual outcome.

## Inspect, step and retry

`run <request-id> --read-only --next-step` executes one handler and persists the
next position. Restart with the same request ID. A failed stage retains its ID
and a retry deadline; fix its cause and rerun when due. Raise `--max-items` for a
preview that exceeds its operation budget. Completed stages are not replayed.

A terminal rerun returns the saved inspection/report. To inspect changed source
data again, submit with a new run ID. Never reset successful preview stages and
turn them into deletion successes.

`run <request-id> --print-only` (also the default real-request mode) prints plans
without approvals, leases, stage writes or child submission. Select one handler
with `--stage purge_profile --community <uuid>`. `--next-step` in stateless mode
inspects the current position; it does not advance it.

Handler details go to stderr; summaries and the final inspection go to stdout.
The persisted report contains stage results, child observations, profile
prerequisites and unsupported gate/drain, global, cache and erasure-verifier work.
An exit code of zero alone is insufficient: a shutdown or a single step can also
return zero. Inspect `status == "completed"`, `completed_at` and the final report.

## Next live testing stage

Run this only when the operator authorizes the next stage. No live rehearsal is
required merely to read the PR or compile its regression tests.

Seed a target, survivor and directly owned agents in member, former and owned
communities. Include the same agent owned by another human in a different
community, shared and exclusive uploads with multiple object versions, content,
account records, profile prerequisites, preserved canvases and Git repositories.
Use each handler's existing regression fixtures as the edge-case checklist.

Exercise the compiled CLI uninterrupted and with `--next-step`, then interruption
between handler success and stage advancement, storage failure/retry, child
submission/adoption, terminal rerun and a second fresh run. Separately exercise
no-community scope, changed ownership, blocked children and operation limits.
Compare product rows and every storage version before/after; only rehearsal and
child-intent/inventory bookkeeping may differ. Do not approve/run child deletions.

Infrastructure-free tests cover routing and mode/identity checks. Ignored
PostgreSQL tests cover idempotent submission and a real handler/coordinator
journey with an S3 transport fake; they are not evidence of a real MinIO or live
relay journey. Follow `crates/buzz-db/TESTING.md` for the isolated database lane.
