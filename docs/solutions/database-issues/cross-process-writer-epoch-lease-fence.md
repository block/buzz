---
module: "Buzz relay database writer-fence cutover"
date: "2026-08-04"
problem_type: "database_issue"
component: "database"
severity: "critical"
symptoms:
  - "The VPS cutover was blocked until restore reconciliation and independent-restore proof were closed."
  - "There was no durable proof that one live relay held the current writer epoch and lease."
  - "Public application routing exposed /_readiness instead of keeping readiness on the internal health listener."
  - "A stale writer was not rejected consistently at the database and external-effect boundaries."
root_cause: "incomplete_setup"
resolution_type: "migration"
related_components:
  - "buzz-relay"
  - "pubsub"
  - "deployment"
  - "health-probes"
tags:
  - "writer-fence"
  - "epoch-lease"
  - "stale-writer"
  - "postgres"
  - "pubsub-guard"
  - "readiness-hardening"
  - "least-privilege"
  - "deployment-cutover"
---

# Cross-process writer epoch/lease fence

## Problem

The Buzz Desktop v0.5.4 rollout could not make the VPS relay safe merely by
installing the release or observing a healthy container. The cutover needed
three separate proofs: the restored backup was independently reconciled, one
relay held current writer authority, and public application traffic could not
use `/_readiness` as an exposed readiness surface.

Without a database-final authority check, an old relay or stale pooled
connection could continue to write durable state after a replacement process
became the intended writer. Database-only enforcement would also leave Redis
publication and external push delivery outside the same authority boundary.

## Root cause

The deployment was incomplete, not merely misconfigured: it lacked the
database writer-fence control plane, the relay lifecycle that acquired and
renewed it, the non-superuser privilege boundary, and a durable deployment
overlay that selected the fenced runtime. The readiness route was also wired
on the public application surface instead of only on the health listener.

## What did not work

1. A release version or a healthy container did not prove writer authority,
   restore eligibility, role hardening, or lease renewal.
2. A process-local epoch, a host-level stop, or a publisher-only fail-closed
   gate did not provide a PostgreSQL-final stale-writer rejection boundary.
3. Keeping the relay runtime role as a superuser would preserve a privilege
   escape around the intended trigger and ownership boundary.
4. Starting the generic Compose model without the writer-fence overlay would
   silently omit the required image and environment.
5. The migration could not be installed by starting a required-fence relay
   before the authority table and functions existed; bootstrap must be
   migration-only with fencing disabled.

## Solution

### 1. Make PostgreSQL the final writer authority

`migrations/0027_writer_epoch_fence.sql` creates the deployment-global
`public.buzz_writer_fence` authority, with a monotonic epoch, holder, mode,
lease expiry, and update time, plus the singleton
`public.buzz_writer_fence_config` enforcement row. Acquisition serializes the
resource and advances the epoch; renewal succeeds only for the matching
resource, epoch, and holder while the lease is valid. The config row, not a
database or session GUC, decides whether durable mutations require a valid
lease; a missing config row fails closed.

The migrations install `ENABLE ALWAYS` DML and `TRUNCATE` guards on the
protected application tables, excluding the fence authority and SQLx
migrations. Migration 0028 additionally installs a deferred commit guard and
the shared-lock effect permit. When the server-side config row is required,
missing, stale, replaced, expired, or fenced authority is rejected by
PostgreSQL. Local development may keep the config row disabled, so the
fail-closed statements in this learning are scoped to the required-fence
deployment mode.

### 2. Bind the relay writer pool to the lease

In required mode, `crates/buzz-db/src/writer_fence.rs` validates the lease and
renewal bounds, acquires the lease at startup, renews it in the background, and
fails closed when renewal or live revalidation is lost. In the same mode, every
writer-pool connection is stamped with the resource, epoch, and holder in its
`after_connect` hook. Production audit writes reuse the fenced writer pool;
test fixtures that create direct pools are not production bypass paths.

### 3. Extend the same boundary to external effects

Migration `0028_writer_epoch_commit_and_effect_fence.sql` adds a
server-authorized effect permit. The permit opens a PostgreSQL transaction,
takes a shared lock on the live fence row, and remains open until the external
operation returns. Epoch acquisition takes the incompatible row lock, so a
takeover cannot linearize during Redis publication or push delivery.

`buzz-pubsub` now uses this permit for Redis publication, presence, and
connection-control mutations. The push runtime uses it for the gateway request.
The event ID and push outbox UUID remain stable retry identities; the code never
turns an ambiguous permit commit into a fresh external identity.

### 4. Separate public liveness from internal readiness

The public application router retains liveness but no longer mounts
`/_readiness`. The health-only listener retains `/_readiness` and checks
PostgreSQL, Redis, and writer-fence state. In the required-fence deployment,
readiness is successful only while the active lease is valid; with fencing
disabled for local development, the fence check is intentionally a no-op. The
response exposes sanitized status rather than the holder or lease token.

### 5. Keep cutover ordering explicit

The operator sequence is documented in
[`docs/writer-fence-cutover.md`](../../writer-fence-cutover.md):

1. Complete backup reconciliation and independent restore proof; stop or
   drain old writers.
2. Run migrations 0027 and 0028 in migration-only mode with fencing disabled and without
   serving traffic.
3. Apply `scripts/writer_fence_hardening.sql`: retain a dedicated non-login
   fence owner, make the relay role non-superuser and non-owner, revoke direct
   authority-table access, and grant only the required fence functions.
4. Set the server-side writer-fence config row to `required=true` and start
   the relay with a 30 second lease and 10 second renewal interval.
5. Verify internal readiness, public readiness absence, multiple renewal
   observations, trigger rejection, and container health.

The durable overlay is
[`deploy/compose/writer-fence.override.yml`](../../../deploy/compose/writer-fence.override.yml).
Manual recreation must include it explicitly. The existing external staging
provenance lock still points at the older upstream v0.5.1 path; it was not
rewritten as part of this cutover and must not be treated as selecting the
fenced overlay automatically.

## Verification

### Repository evidence

The current integration worktree contains the migration, runtime lease
lifecycle, connection stamping, external-effect guards, role-hardening SQL,
health-router split, and deployment overlay. The relevant source and tests
include:

- [`migrations/0027_writer_epoch_fence.sql`](../../../migrations/0027_writer_epoch_fence.sql)
- [`migrations/0028_writer_epoch_commit_and_effect_fence.sql`](../../../migrations/0028_writer_epoch_commit_and_effect_fence.sql)
- [`crates/buzz-db/src/writer_fence.rs`](../../../crates/buzz-db/src/writer_fence.rs)
- [`crates/buzz-db/src/lib.rs`](../../../crates/buzz-db/src/lib.rs)
- [`crates/buzz-relay/src/router.rs`](../../../crates/buzz-relay/src/router.rs)
- [`crates/buzz-pubsub/src/lib.rs`](../../../crates/buzz-pubsub/src/lib.rs)
- [`crates/buzz-relay/src/push_runtime.rs`](../../../crates/buzz-relay/src/push_runtime.rs)
- [`scripts/writer_fence_hardening.sql`](../../../scripts/writer_fence_hardening.sql)

The deployment session recorded remote passes for formatting, locked package
checks, writer-fence unit tests, migration tests, and pubsub unit tests. This
is session-level verification rather than a repository-tracked CI receipt. The
full relay test suite was not runnable on the build host because
`libssl-dev`/`openssl.pc` was missing; this is an environment limitation, not a
claimed green full-suite result.

The local integration worktree subsequently added migration 0028 and validated
the new boundaries against a temporary PostgreSQL 17 instance:

- a long transaction that loses the epoch fails at commit and leaves no row;
- an external-effect permit blocks an expired-lease takeover until the permit
  transaction commits;
- the hardening script drains a live privileged `buzz` session, then proves the
  replacement role is non-superuser and owns no public relation.

These are local/candidate proofs only. Migration 0028 and the revised hardening
script have not been applied to the production VPS in this worktree.

### Desktop and VPS receipts

The paths below are historical handoff references. They are not refreshed by
the local fixes in this worktree; in particular, this worktree did not mutate
the production VPS and does not treat an older VPS receipt as proof that
migration 0028 is live.

The Desktop update is recorded in
`C:\Users\CEDRF\AppData\Local\Buzz-update-receipts\20260803-212904-v0.5.4\desktop-success-20260804-002718.md`.
It verifies the signed installer, protected-profile backup, 0.5.4 executable,
shortcut, cold launch, and retained rollback installation.

The independent restore precondition is recorded in
`C:\Users\CEDRF\AppData\Local\Buzz-update-receipts\20260803-212904-v0.5.4\vps-g2-b2-independent-restore-20260804-141144Z.md`:
generation `20260804T133729Z-jt-buzz-staging`, the corresponding B2 attempt,
and restore result `PASS`.

The post-cutover VPS receipt is
`C:\Users\CEDRF\AppData\Local\Buzz-update-receipts\20260803-212904-v0.5.4\vps-writer-fence-success-20260804-1900Z.md`.
It records the following live evidence:

- official v0.5.4 base digest, fenced runtime image
  `jt-buzz-writer-fence:20260804`, and running/healthy relay container;
- SQLx migration version 27, the pre-hardening database default enabled,
  dedicated fence owner, non-superuser `buzz` runtime role, and denied direct
  fence-table read. This historical receipt does not prove the new
  server-authoritative config-row implementation.
- 92/92 `ENABLE ALWAYS` guard triggers, including `events`, with an unfenced
  DML transaction rejected and rolled back;
- live `buzz-relay` epoch 1 and active lease observed in two snapshots 12
  seconds apart;
- public `127.0.0.1:3300/_readiness` returning 404, public liveness returning
  200, and internal `8080/_readiness` returning 200 with `{"status":"ready"}`;
- watchdog health PASS and durable remote overlay
  `/opt/jt-buzz-staging/ops/buzz/compose.writer-fence.yml`, whose SHA-256 is
  `39814dcdf5dc84ead536ee31c5d44b76dcae935ae51ab2ccd6bf5f684e3a5618`.

The receipt records the fenced runtime image by a VPS-local tag but does not
prove registry publication. Treat that image as local to this handoff unless a
separate publication receipt exists; a fresh host therefore requires an
explicitly authorized rebuild or publication step. A future manual Compose
recreation must include the durable overlay; the old v0.5.1 staging lock is not
sufficient.

## Bounded status and residual hardening

The live receipt proves the configured cutover and the basic guard, lease,
readiness, backup, and health observations. A separate read-only integrity
review found that this is not yet a proof of an adversarially complete fence;
these items remain open before calling the writer-fence design fully closed:

- **Server authority:** the integration now stores requiredness in the
  server-side `buzz_writer_fence_config` row and removes the trigger's
  dependency on the session GUC. The regression test must pass locally and on
  the migration target, and the hardening SQL must be applied there; the old
  live receipt does not close this gate.
- **Transaction boundary:** resolved in the local candidate by migration 0028's
  deferred commit trigger and the long-transaction adversarial test. It still
  needs the same migration applied and tested on the migration target.
- **External effects:** resolved in the local candidate by the transaction-held
  Redis/HTTP permit and stable retry identities. This is not live proof until
  the fenced runtime image and migration 0028 are deployed together.
- **Role/session rotation:** resolved in the local candidate by the drain loop,
  explicit statistics-snapshot refresh, and post-rotation ownership/session
  audit. It still needs to be executed during the authorized VPS cutover.
- **Mode and deployment alignment:** invalid or missing process configuration
  must fail closed when the server-side config row requires fencing. The
  supported generic Compose wrapper does not select the durable overlay
  automatically.
- **Mesh and health network:** raw mesh Redis registry writes need the same
  fence or an explicit disabled/scope decision, and the health listener's
  `0.0.0.0` binding needs an external-port denial test if it is described as
  internal.

Until migration 0028, the revised hardening SQL, and their live proofs are
applied to the target, the correct status is: **Desktop update complete;
writer-fence corrections validated locally; VPS release remains on HOLD.**

## Prevention

- Treat migrations 0027/0028, hardening SQL, the cutover runbook, and the Compose
  overlay as one release contract.
- Require independent restore proof, old-writer drain, role/ownership audit,
  migration version 28, active epoch and repeated renewal evidence, public
  readiness absence, internal readiness, and container health before promotion.
- Keep database triggers as the durable backstop and hold the effect permit
  across Redis or external effects; do not replace the boundary with a
  process-local preflight gate.
- Attach the guard to future non-partition application tables; current child
  partitions inherit the migration's trigger coverage, but future
  non-partition table creation is not automatically covered.
- Keep the staged overlay selection explicit until the external staging and
  provenance lock is formally moved to the fenced v0.5.4 integration.

## Related source

Use the existing cutover runbook for procedure and this learning for the
failure mode and durable constraints. Do not create a second live receipt or
copy the operator secret material into the repository.
