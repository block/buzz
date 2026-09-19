# Durable data backfills: a specification

`draft`

## Overview

This section is non-normative. It gives the mental model for the requirements
that follow.

A Buzz data backfill repairs existing PostgreSQL rows after a compatible schema
is available. It may start automatically with the relay or manually while the
relay serves traffic. If a worker stops, another worker resumes from committed
progress.

Think of each backfill as one durable PostgreSQL row keyed by a stable ID:

1. The application registers the stable ID.
2. Starting the backfill captures a finite upper bound.
3. A worker takes an exclusive claim.
4. Each transaction changes target rows and advances the checkpoint together.
5. Workers repeat bounded batches until they reach the upper bound.
6. Validation proves the result before the row becomes `completed`.

PostgreSQL is the authority throughout this flow. Queues and timers may wake a
worker, but they do not own work or progress. The admin API and client display
the PostgreSQL state; they do not keep another copy of the lifecycle.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described in BCP 14 when they appear in
all capitals.

## Purpose and scope

This specification covers:

- durable registration and execution of bounded data backfills;
- exclusive claims, crash recovery, retry, pause, and validation;
- atomic target mutations and checkpoint progress;
- independent schema-migration and automatic-backfill controls;
- readiness behavior in automatic and manual modes;
- an authorized, auditable admin API and client; and
- conformance tests at production transaction, startup, and admin seams.

It does not define:

- schema migration mechanics or a generic migration framework;
- a Nostr event kind or other wire protocol;
- a distributed queue as durable state;
- exactly-once worker code outside a PostgreSQL transaction;
- one universal batch size, timeout, or retry schedule;
- a way for a backfill to make an incompatible application schema safe; or
- destructive controls that erase, rewind, or force completion.

Backfill definitions are application code, not runtime-authored migrations.

## Core model

### Terms

| Term | Meaning |
|---|---|
| **Definition** | Application code that declares an ID, ordering key, batch bound, mutation, and validation. |
| **Stable ID** | The only identity for execution, administration, validation, completion, and readiness. |
| **Upper bound** | The inclusive, definition-typed end of the finite source set. It is captured once. |
| **Checkpoint** | The greatest source position whose required mutation is durably committed or was deterministically unnecessary. |
| **Claim** | Temporary, exclusive PostgreSQL authority for one worker to advance a backfill. |
| **Generation** | A monotonically increasing value that fences work from a stale claim. |
| **Validation** | Definition-specific proof that work through the upper bound has the required postcondition. |
| **Required automatic backfill** | A stable ID that the deployed application includes in automatic startup readiness. |

### PostgreSQL authority

PostgreSQL MUST be the sole authority for lifecycle, ownership, progress,
retry eligibility, diagnostics, validation, and completion. A queue, timer,
cache, notification, or in-process registry MAY wake a worker. Loss, delay,
duplication, or replay of that signal MUST NOT affect correctness. A worker
MUST re-read PostgreSQL before acting.

The row for a stable ID MUST remain compact. It records the lifecycle, immutable
bound, monotonic checkpoint, claim and generation, bounded retry and diagnostic
state, validation, timestamps, and audit correlation. Bounded definition
metadata MAY be stored for diagnostics. Detailed history belongs in the audit
facility, not an unbounded per-batch journal. Telemetry is a projection, not
another state store.

### Stable identity and registration

Each definition MUST declare a stable ID. A stable ID MUST map to exactly one
durable row and at most one readiness obligation across builds and deployments.
Validated completion MUST be permanent for that ID.

Registration and reconciliation MUST be idempotent upserts by stable ID.
Repeated registration MUST NOT create another execution. Registration signals
may arrive late, more than once, or out of order; the durable state MUST still
converge on one row and one readiness obligation. For registration-owned
diagnostic metadata, the last registration wins.

Registration order, revision strings, build metadata, and version metadata
MUST NOT select a runnable definition or establish execution precedence. They
MUST NOT create another row, checkpoint, completion, or readiness obligation.

Every process allowed to work on an active ID MUST implement the same behavior:
the same source set, ordering and checkpoint meaning, mutation, and validation
contract. A material change to any of those semantics MUST use a new stable ID.
Registering different behavior under an active ID is operator error; the
orchestrator MUST NOT compare or choose between definitions.

### Rolling deployments

A process MUST reconcile its definitions by stable ID before claiming work.
Any process with a compatible local definition MAY claim eligible work,
regardless of registration order or diagnostic metadata. A process MUST fail
closed if it does not know the definition or cannot safely interpret the row's
bound, checkpoint, lifecycle, and completion contract.

Deploying another compatible process does not revoke an existing claim. Its
owner MAY commit while the claim and generation remain valid. If the deployment
cannot accept that commit, its semantics are materially different and require a
new stable ID. Deployment order, binary age, and metadata MUST NOT act as fences.

An exclusive claim authorizes one compatible worker. Generations reject stale
owners. These mechanisms MUST NOT establish definition precedence.

## Lifecycle and operator actions

One state machine governs automatic and manual execution:

| State | Meaning | Next states |
|---|---|---|
| `pending` | Registered but not started; no bound exists. | `running` |
| `running` | Started and eligible for a worker claim, subject to retry backoff. | `paused`, `blocked`, `failed`, `validating` |
| `paused` | Operator intent prevents claims and fences current work. | `running`, `validating` |
| `blocked` | A prerequisite or data condition needs operator remediation. | `running`, `validating` |
| `failed` | Execution exhausted bounded retries, or validation failed. | `running`, `validating` |
| `validating` | Mutation reached the bound and is eligible for validation. | `paused`, `completed`, `failed`, `blocked` |
| `completed` | Validation succeeded. This state is terminal. | none |

The supported operator actions are:

| Action | Contract |
|---|---|
| `start` | From `pending`, atomically capture the upper bound and enter `running` before dispatching work. |
| `pause` | From `running` or `validating`, atomically enter `paused`, advance the generation, and invalidate the claim. |
| `resume` | From `paused`, enter `running`, or `validating` when no admitted work remains. |
| `retry` | From `blocked` or `failed`, continue at the existing bound and checkpoint. |
| `validate` | For `pending`, return `current-state`. For any other row, use the read-only validation contract below. |

Repeated `pause` and `resume` requests MUST converge on the requested state.
Neither action changes the bound or checkpoint. There is no separate retrying
or cancellation state. Retry eligibility and delay are attributes of `running`;
`pause` is the safe interruption command.

An in-flight batch either commits before `pause` or fails the generation fence
and rolls back. Previously committed batches remain committed.

### Retry, blockage, and failure

The engine MUST durably classify an execution error before reporting it as
handled. Retryable errors remain `running` without a claim until bounded
backoff permits another claim. Automatic retries MUST be bounded and
end in a terminal `failed` or `blocked` disposition instead of a hot loop.

An unmet prerequisite or data invariant enters `blocked` with a bounded,
actionable diagnostic. Exhausted execution errors enter `failed`. Validation
failure enters `failed`, unless it identifies an unmet prerequisite and enters
`blocked` instead.

`retry` is fresh operator intent. It MAY clear bounded attempt accounting and
stale diagnostics. It MUST preserve history, the bound, and the checkpoint, and
MUST NOT bypass validation.

### Validation and completion

Reaching the upper bound MUST enter `validating`, not `completed`. Validation
MUST read authoritative PostgreSQL state, cover the postcondition through the
bound, and be safe to repeat after a crash.

Only successful validation MAY transition the row to `completed`. The
completion transaction MUST verify the lifecycle and generation so a stale
validator cannot complete a paused or retried run. Operators cannot select
`completed`, and completed state MUST remain immutable.

## Execution safety

### Finite source set and checkpoint

Before the first mutation, `start` MUST capture an upper bound in the same
transaction that initiates execution. The definition MUST state how the bound
represents an empty source set. Once captured, the bound MUST NOT be changed or
reinterpreted for that stable ID.

The definition MUST use a checkpoint type with a total, stable ordering. The
checkpoint MUST be inclusive. Before the first source item commits, the
checkpoint MAY be absent; absence means a position before all admitted work,
not a worker-supplied text sentinel.

The checkpoint MUST NOT move backward or beyond the upper bound. A worker MUST
select positions strictly after the checkpoint and no later than the bound. If
the ordering key is not unique, the definition MUST use a stable compound key
that cannot ambiguously skip or repeat tied rows.

Rows created beyond the upper bound MUST be handled by normal write paths or a
new backfill ID. Application reads and writes MUST remain correct for both
repaired and unrepaired rows during execution.

Rationale: the upper bound makes historical repair finite and separates it from
live writes.

### Bounded batches

Each definition MUST declare a finite maximum amount of work per batch. The
production selection and mutation path MUST enforce it. Any work that could
grow with the source set MUST have that bound or a stricter finite bound.

Batch transactions and lock-holding intervals MUST have bounded duration.
Workers MUST observe cancellation, timeout, and pause intent at transaction and
batch boundaries. They MUST NOT start another batch after observing that intent,
and an interrupted transaction MUST roll back.

If a work, transaction, lock, or interruption bound cannot be honored, the
engine MUST commit neither partial mutations nor a checkpoint. It MUST apply
the bounded failure policy instead of widening the batch or continuing without
a bound.

### Atomic mutation and progress

Every batch MUST commit its target mutations and checkpoint advance in one
PostgreSQL transaction. A mutation, checkpoint, ownership, commit,
cancellation, or timeout failure MUST roll back both.

Rationale: this transaction keeps committed mutations from being hidden behind
an older checkpoint and keeps the checkpoint from claiming rolled-back work.
Definitions SHOULD still make mutations idempotent as defense in depth.

Side effects that cannot join the transaction are outside the backfill commit.
They MUST NOT be required for the checkpoint to mean complete.

### Exclusive claims and generation fencing

A backfill MUST NOT have more than one current claim. Acquisition, renewal,
release, and takeover MUST be PostgreSQL state transitions. Each new claim or
takeover MUST advance the generation. An admin transition that invalidates
in-flight work MUST also advance it.

Every transaction that changes target data or progress, or commits a validation
or lifecycle result that can advance execution, MUST verify before commit that:

- the lifecycle permits the operation;
- the presented owner holds the current claim;
- the presented generation equals the current generation; and
- the claim remains valid.

Failure of any check MUST reject the entire transaction. A check made only
before work starts is insufficient because a paused, expired, or replaced owner
may finish after takeover.

Claims MUST have bounded validity and require renewal. Takeover MUST eventually
be possible after renewal stops. PostgreSQL time and state decide validity;
worker-local clocks do not grant authority.

### Release, crash, and takeover

A worker MAY release a claim while the row remains `running`. A crash leaves
either a committed batch or no batch effect. After the abandoned claim expires,
another worker advances the generation and resumes strictly after the committed
checkpoint.

The old worker may keep computing, but its later transaction MUST fail the
generation check. Takeover MUST NOT rewind the checkpoint, recapture the bound,
or infer progress from worker memory.

## Configuration and readiness

Schema auto-migration and automatic backfills are independent controls. All
four configurations are valid:

| Schema auto-migration | Automatic backfills | Required behavior |
|---|---|---|
| off | off | The relay MUST run no migrations, MUST NOT start `pending` backfills, and MUST NOT add a backfill readiness gate. Already-started manual work MUST remain recoverable. The installed schema and application MUST serve safely with incomplete data. |
| off | on | The relay MUST run no migrations. With a compatible installed schema, it MUST start required backfills and remain unready until they reach `completed`. Missing required schema is a schema-compatibility failure; a backfill MUST NOT replace migration. |
| on | off | The relay MUST run migrations, but MUST NOT start `pending` backfills or add a backfill readiness gate. Operators run them manually while the application serves mixed repaired and unrepaired data. Already-started work MUST remain recoverable. |
| on | on | The relay MUST run migrations, then required backfills, and remain unready until every required backfill reaches `completed`. |

In automatic mode, every required `pending`, `running`, `paused`, `blocked`,
`failed`, or `validating` row MUST keep the serving gate closed. The relay MUST
register the complete set of required stable IDs before evaluating that gate.
A restart MUST reconstruct the gate from PostgreSQL; late discovery MUST NOT
create a ready interval.

In manual mode, incomplete backfills MUST NOT make the relay unready. This is
safe only when application reads and writes work correctly before, during, and
after the repair. A schema change that cannot serve safely until its backfill
finishes does not conform to manual mode.

Turning automatic backfills off prevents automatic initiation of `pending`
rows. It does not abandon `running` or `validating` work. Workers MUST recover
or take over already-started work after failure without adding a readiness gate;
an operator MAY pause it.

Backfill readiness is only one input to the serving decision. Database, cache,
deletion-fence, shutdown, and schema-safety checks remain independent.

## Admin and client contract

The relay admin API is the external boundary. The client and operator tools
MUST project its PostgreSQL-backed state and invoke the lifecycle actions above;
they MUST NOT keep another lifecycle or synthesize outcomes.

### Read model and honest projection

Authorized list and detail reads MUST expose:

- stable IDs and bounded informational definition metadata;
- lifecycle, bound, checkpoint-derived progress, ownership, retry eligibility,
  and timestamps;
- bounded execution and validation diagnostics;
- the latest explicit validation disposition, outcome, and time; and
- participation in the automatic readiness gate.

Unknown totals, unavailable diagnostics, and failed or stale reads MUST remain
unknown. The UI MUST NOT render them as zero, complete, healthy, or offline.
The server MUST derive current status from PostgreSQL on each read. A client MAY
cache for presentation, but a failed refresh does not make cached data current.

### Control idempotency

The API and client MUST support exactly the lifecycle actions listed above.
They MUST be safe under retries, duplicate delivery, concurrent operators, and
a response lost after commit. Repeating one logical request MUST return its
original result or the converged state without duplicating a transition,
recapturing a bound, reusing a generation, or starting another owner.
Conflicting requests MUST return a typed conflict with current server state.

In manual mode, `start` is the durable intent that separates recoverable work
from registered `pending` work that automatic configuration MUST leave alone.

The normal API MUST NOT expose reset, checkpoint rewind, bound changes, record
deletion, force completion, unfenced claim release, or generic cancellation.
Incompatible work uses a new stable ID. Destructive recovery requires a
separate specification.

### Explicit validation

`validate` requests definition-owned, read-only validation against PostgreSQL.
It is not a second lifecycle or an operator-selected completion transition.

The API MUST return exactly one of these typed outcomes:

| Outcome | Meaning |
|---|---|
| `success` | Validation ran and the postcondition held. |
| `failure` | Validation ran and rejected the state, with a bounded diagnostic. |
| `current-state` | Validation did not run or has no result yet; the response includes the lifecycle and reason without implying success or failure. |

A `pending` row MUST return `current-state` without running validation. Any
other row MAY be validated. When the row is `validating` and
completion preconditions hold, the request MAY drive the same generation-fenced
completion transition used by the lifecycle. In every other state, validation
is diagnostic and MUST NOT change the lifecycle, bound, checkpoint, generation,
or target data.

Revalidating a completed row MUST report a current failure as `failure` while
leaving immutable completion and progress unchanged.

Only one validation invocation per stable ID MAY run at a time. Duplicate
delivery of one logical request MUST return its original bounded outcome or a
correlated `current-state` result. Concurrent requests MUST join the active
invocation or return `current-state`. After a lost response, retrying the same
request MUST recover the outcome. The client MUST retry or refresh; it MUST NOT
infer the outcome.

Diagnostic validation MUST persist only its bounded outcome, request
correlation, and audit record. It MUST NOT require or create an execution claim.
It MUST remain serialized, operator-authorized, read-only, and unable to change
immutable execution state.

Accepted, rejected, coalesced, and completed validation requests MUST be audited
with request correlation, actor, stable ID, evaluated lifecycle, and bounded
outcome. Audit recording MUST NOT turn validation into a target-data write.

### Authorization and audit

Backfills are deployment-wide operational state. Reads and controls MUST
require an explicit deployment-operator capability at the relay admin boundary.
Community ownership, channel administration, and membership MUST NOT grant this
authority. A protected network boundary MAY supply equivalent authority, but
the deployment MUST NOT claim individual attribution it does not have.

Every accepted or rejected control attempt MUST be audited with the actor or
authority source, stable ID, request correlation, prior and resulting state,
claim generation when relevant, time, and a bounded outcome. An accepted state
change and its audit record, or durable intent to append that record, MUST commit
atomically. An unaudited change is not success.

Diagnostics in the API, audit, and logs MUST be bounded and redacted. They MUST
NOT expose raw source rows, credentials, unbounded database errors, or query
text. Detailed diagnostic reads SHOULD be observable without logging row data.

### Observability

Operators MUST be able to distinguish:

- lifecycle counts and readiness-gating disposition;
- claim acquisition, renewal, loss, takeover, and stale-owner rejection;
- attempted and committed work, checkpoint movement, and lack of progress;
- retryable errors, blockage, exhausted failure, and validation outcomes; and
- execution and validation duration.

Metric labels MUST use bounded vocabularies. Stable IDs, checkpoints, row
values, diagnostics, and actor IDs belong in authorized views and structured
logs, not labels. Missing telemetry is unknown. Metrics and logs MUST reconcile
with PostgreSQL state and MUST NOT authorize transitions.

## Conformance tests

Tests MUST exercise production transaction, startup, admin, and client seams.
They MUST be falsifiable: removing the relevant bound, atomic write, fence,
authorization check, or readiness input MUST fail a test. Test-only lifecycle
helpers do not prove conformance.

At minimum, the suite covers:

1. **Claim race.** Concurrent workers contend for one row. Only one worker owns
   the current claim and commits under its generation; eventual takeover
   preserves the checkpoint.
2. **Bounded batch.** A source set larger than one batch advances only within
   the declared batch bound before releasing the transaction. Cancellation or
   timeout rolls back target mutations and checkpoint. Removing production
   bound enforcement fails the test.
3. **Rollback.** Failure after mutation but before commit, including checkpoint
   failure, leaves target data and checkpoint unchanged.
4. **Restart.** Termination before and after batch commit, and during validation,
   rebuilds progress from PostgreSQL without recapturing the bound.
5. **Stale owner.** Pause, expiry, or takeover while a worker is in flight
   rejects its mutation, checkpoint, validation, and completion at commit.
6. **Rolling deployment.** Two compatible processes register the same stable ID
   in either order and retain one row and readiness obligation. Either MAY win
   an exclusive claim. After takeover, generation fencing rejects the prior
   owner's writes. A process that does not know the stable ID cannot claim. The
   test MUST NOT choose by registration order or metadata, or assert definition
   precedence.
7. **Validation.** Reaching the bound cannot complete without validation.
   Failure is durable and repeatable; only success reaches immutable completion.
8. **Bounded failure.** Persistent errors reach `failed` or `blocked` instead of
   retrying forever. Operator retry preserves the bound and checkpoint.
9. **Admin authorization.** Unauthorized reads and controls return no protected
   state and perform no mutation. Community roles grant no authority.
10. **Admin idempotency.** Duplicate, concurrent, and lost-response requests do
    not duplicate starts, reuse generations, or recapture bounds. Conflicts
    return current state.
11. **Explicit validation.** Start with a `completed` row and no execution claim.
    Through the production API and client, run validation, persist and display
    its current typed outcome, and write its audit record. It creates no claim
    and does not change the generation, target data, lifecycle, bound,
    checkpoint, or completion. The API and client also render all typed outcomes
    for non-`pending` rows. Unauthorized, duplicate, concurrent, and
    lost-response requests follow the contract above. Removing serialization,
    read-only behavior, or an immutable-state guard fails the test.
12. **Client projection.** The UI preserves unknown and failure states and sends
    only supported controls.
13. **Configuration matrix.** All four configurations run through real startup
    and readiness paths. Automatic mode gates until validation, manual mode does
    not add the gate, and schema safety remains independent.

PostgreSQL concurrency tests SHOULD use controlled transaction barriers so
losing and stale commits are observed rather than inferred from timing.

## Component mapping

| Responsibility | Buzz component |
|---|---|
| Stable-ID registration, lifecycle, durable store, claims, fencing, checkpoints, retry, and validation | The new `buzz-backfill` crate owns the domain and persistence. |
| Database and transaction primitives for atomic mutation and checkpoint progress | `buzz-db` provides narrow primitives. It does not own backfill records, policy, or orchestration, and it does not expose its connection pool. |
| Discovery, automatic execution, configuration, startup ordering, and readiness | `buzz-relay` composes the engine with startup and serving coordination. |
| Authorized reads, lifecycle actions, validation, audit, conflicts, and diagnostics | Relay deployment-admin interfaces expose the contract; tools use them instead of accessing tables. |
| List, detail, progress, diagnostics, readiness, and controls | The desktop/admin client projects relay state and never stores another lifecycle. |

This keeps the repository's focused-crate architecture: the relay orchestrates,
the backfill crate owns its domain, and the database crate provides general
transaction infrastructure. Backfill administration remains an operator admin
concern; it does not add a Nostr event or NIP.
