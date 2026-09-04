# Durable data backfills: a specification

`draft`

## Abstract

This document specifies how Buzz performs durable data backfills outside schema
migrations. A backfill transforms existing PostgreSQL data after a compatible
schema is available, may run automatically during relay startup or manually
while the relay serves traffic, and survives process failure without losing or
duplicating committed progress.

The design has one durable lifecycle. PostgreSQL records the backfill identity,
immutable scan bound, monotonic checkpoint, state, current claim generation,
bounded retry disposition, validation result, and diagnostics. Workers may be
woken by queues or caches, but those systems carry no authority. Every data
mutation and its checkpoint advance commit in one PostgreSQL transaction, and
every owner must present the current generation in that transaction.

The specification also defines the operator contract. The authenticated relay
admin surface and its client projection expose the same PostgreSQL-backed state
and safe lifecycle controls. Manual execution is therefore a supported operating
mode, not a database-side escape hatch.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are to be interpreted as described in BCP 14 when they appear in
all capitals.

## Scope and non-goals

This specification covers:

- durable registration and execution of bounded data backfills;
- exclusive work claims and takeover after a worker disappears;
- atomic progress, retry, pause, validation, failure, and completion;
- independent configuration of schema auto-migration and automatic backfills;
- readiness behavior in automatic and manual modes;
- an authorized, auditable relay admin API and client control surface; and
- conformance evidence at the transaction, concurrency, startup, and admin
  seams.

It deliberately does not define:

- schema migration mechanics or a generic migration/version framework;
- a Nostr event kind, filter extension, or other wire protocol;
- a distributed queue as durable state;
- exactly-once execution of worker code outside a transaction;
- a universal batching, timing, or retry policy;
- a way to make an incompatible application schema safe by running a backfill;
  or
- an operator control that erases history, rewinds progress, changes a captured
  bound, or declares success without validation.

Backfill definitions are application code. This protocol coordinates their
durable execution; it does not turn arbitrary data transformations into a
runtime-authored migration language.

## Terminology

- **Definition** — application code for one data transformation and its
  validation. It declares a stable identity, a definition version, a typed
  ordering key, mutation logic, and validation logic.
- **Backfill** — the durable execution record for one definition identity and
  version.
- **Upper bound** — an inclusive, definition-typed boundary that makes the
  admitted source set finite, captured once before mutation and never changed.
- **Checkpoint** — the greatest source position whose required mutations are
  durably committed for this backfill.
- **Claim** — PostgreSQL state granting one worker temporary exclusive authority
  to advance a backfill.
- **Generation** — a monotonically increasing fencing value stored with the
  backfill. A claim is authorized under one generation, and only that generation
  may commit worker results.
- **Attempt** — one claimed interval of execution ending in progress, release,
  loss of ownership, pause, blockage, or error.
- **Validation** — the definition-specific proof that all work through the upper
  bound has the required postcondition.
- **Required automatic backfill** — an identity-version pair declared by the
  deployed application as participating in automatic startup readiness.
- **Automatic mode** — the relay initiates required automatic backfills and
  gates serving on their validated completion.
- **Manual mode** — automatic initiation and readiness gating are disabled;
  operators use the admin surface to control backfills, and already-initiated
  work remains recoverable after failure.

## Authority and durable record

PostgreSQL is the sole authority for lifecycle state, ownership, progress,
diagnostics, validation, and completion. A queue, notification, timer, cache, or
in-process registry MAY wake a worker or reduce polling, but loss, duplication,
delay, or replay of that signal MUST NOT change correctness. A worker always
re-reads PostgreSQL before acting.

The durable record MUST remain compact. It contains only the information needed
to establish:

- definition identity and version;
- lifecycle state;
- immutable upper bound and monotonic checkpoint in the definition's declared
  type;
- current claim identity, generation, and validity;
- bounded retry eligibility and a bounded diagnostic summary;
- validation disposition; and
- lifecycle timestamps and audit correlation.

Detailed history belongs in the existing audit facility rather than an
unbounded per-batch journal. Operational telemetry is a projection of durable
state and attempts, never a second lifecycle store.

## Identity, bounds, and checkpoints

### Stable, versioned identity

A definition identity MUST be stable across builds and deployments. Its version
MUST change whenever the source set, ordering, mutation semantics, or validation
postcondition changes incompatibly. An implementation MUST NOT reuse an
identity-version pair for different semantics.

The identity-version pair names one logical execution. Validated completion is
permanent for that pair. Running a materially different transformation requires
a new version; it does not reset the old record.

### Immutable upper bound

Before the first mutation, the engine MUST capture an upper bound in PostgreSQL.
The definition MUST specify how that bound represents an empty admitted source
set. Bound capture and the transition that first initiates execution MUST be
atomic. Once captured, the bound MUST NOT be increased, decreased, or
reinterpreted for that identity-version pair.

The bound makes the execution finite and separates historical repair from live
writes. Rows created beyond it are handled by normal application write paths or
a later definition version. Schema and application behavior MUST remain correct
for both repaired and not-yet-repaired rows during this overlap.

### Monotonic typed checkpoints

Each definition declares a checkpoint type with a total, stable ordering. The
checkpoint MUST be compared using that type, not a lossy textual convention.
Its meaning is inclusive: after commit, every admitted source item at or below
the checkpoint has had its required mutation committed or has been
deterministically classified as requiring no mutation.

Before any source item is committed, the checkpoint MAY be absent. Absence is
the initial position before every admitted source position; it is not a textual
sentinel supplied by a worker.

A checkpoint MUST NOT move backward or beyond the immutable upper bound. A
worker MUST derive its next work from positions strictly after the committed
checkpoint and no later than the upper bound. If the ordering key alone is not
unique, the definition MUST use a stable compound ordering that does not skip or
repeat tied rows ambiguously.

## Transaction and ownership invariants

### Mutation and checkpoint atomicity

Every batch's data mutations and checkpoint advance MUST commit in the same
PostgreSQL transaction. If any mutation, checkpoint write, ownership check, or
commit fails, the entire batch MUST roll back.

This yields two required properties:

- a committed mutation is never hidden behind an older committed checkpoint;
  and
- an advanced checkpoint never claims a mutation that rolled back.

Definitions SHOULD make mutations idempotent as defense in depth, but
idempotence is not a substitute for the shared transaction. Side effects that
cannot participate in that transaction are outside the backfill commit and MUST
NOT be required to interpret the checkpoint as complete.

### Exclusive claims and generation fencing

At most one current claim exists for a backfill. Claim acquisition, renewal,
release, and takeover are PostgreSQL state transitions. Each successful new
claim or takeover advances the generation monotonically. An administrative
transition that must invalidate in-flight work also advances it.

Every worker transaction that mutates target data, advances progress, or commits
a worker-driven lifecycle result MUST verify all of the following against
PostgreSQL before commit:

- the lifecycle permits the operation;
- the presented owner holds the current claim;
- the presented generation equals the current generation; and
- the claim remains valid.

Failure of any check rejects the entire transaction. Checking only before work
begins is insufficient: a paused, superseded, or expired owner may finish after
a takeover. Generation verification at the commit seam is the fence that makes
that result stale and harmless.

A claim has bounded validity and requires renewal. Its exact policy is an
implementation choice, but takeover MUST eventually be possible after renewal
stops. PostgreSQL time and state decide claim validity; worker-local clocks do
not grant authority.

## Lifecycle

One state machine governs automatic and manual execution:

| State | Meaning | Permitted next states |
|---|---|---|
| `pending` | Registered but not yet initiated; no bound has been captured. | `running` |
| `running` | Durably initiated and eligible for worker claims, possibly after bounded backoff. | `paused`, `blocked`, `failed`, `validating` |
| `paused` | Operator intent forbids new claims and fences current work. | `running`, `validating` |
| `blocked` | A declared prerequisite or data condition prevents safe execution without operator remediation. | `running`, `validating` |
| `failed` | Execution exhausted its bounded retry policy, or validation rejected the result. | `running`, `validating` |
| `validating` | Mutation reached the upper bound and is eligible for a validation claim. | `paused`, `completed`, `failed`, `blocked` |
| `completed` | Validation succeeded for the immutable bound. Terminal. | none |

No parallel retrying or cancellation state machine exists. Retry delay and
eligibility are attributes of `running`; pause is the safe interruption
primitive.

### Crash, release, and takeover

An orderly worker that has more work MAY release its claim while the backfill
remains `running`. A crash leaves either a fully committed batch or no batch
effect. When the abandoned claim is no longer valid, another worker advances the
generation and resumes strictly after the committed checkpoint.

The old worker may continue computing, but its later transaction is rejected by
the generation fence. Takeover MUST NOT rewind the checkpoint, recapture the
upper bound, or infer progress from worker memory.

### Retry, blocked, and failed

Execution errors MUST be durably classified before an attempt is reported as
successfully handled. Retryable failures remain `running` without a claim until
bounded backoff permits another claim, and automatic attempts are bounded. The
policy MUST have a terminal disposition; persistent failure must not create an
unbounded hot loop.

A deterministic unmet prerequisite or data invariant enters `blocked` with a
bounded, actionable diagnostic. Exhausted retryable execution errors enter
`failed`. Validation rejection also enters `failed` unless it identifies an
explicit unmet prerequisite, in which case it MAY enter `blocked`.

Operator `retry` is fresh intent to continue from the existing checkpoint and
upper bound. It returns to `running`, or to `validating` if no admitted work
remains. It MAY clear bounded attempt accounting and stale diagnostics, but MUST
NOT erase history, rewind progress, or bypass validation.

### Pause and resume

Pause MUST atomically set `paused`, advance the generation, and invalidate any
current claim. An in-flight batch that has not committed therefore loses its
authority and rolls back. Already committed batches remain committed.

Resume returns to `running`, or to `validating` if no admitted work remains. It
does not itself claim work and does not alter the upper bound or checkpoint.
Repeated pause or resume requests MUST be safe and converge on the requested
state.

### Validation and completion

Reaching the upper bound is not completion. The engine MUST first enter
`validating` durably, then run definition-specific validation against
authoritative PostgreSQL state. Validation MUST cover the postcondition through
the immutable upper bound and MUST be safe to repeat after a crash.

Only successful validation may transition to `completed`. A completion write
MUST verify the current lifecycle and generation so a stale validator cannot
complete a retried or paused run. `completed` MUST be immutable and MUST NOT be
available as an operator-selected state.

## Configuration and readiness

Schema auto-migration and automatic backfills are independent controls. All four
combinations are valid and MUST have the following behavior:

| Schema auto-migration | Automatic backfills | Required startup and readiness behavior |
|---|---|---|
| off | off | Run no schema migration and do not automatically initiate `pending` backfills. Backfills do not gate readiness. Already-initiated manual work remains recoverable. The installed schema and application MUST already be compatible with serving before incomplete backfills. Independent schema-safety checks may still fail startup or readiness. |
| off | on | Do not run migrations. If the installed schema is compatible, initiate required backfills and keep serving unready until they validate as `completed`. Missing required schema is a schema-compatibility failure; a backfill MUST NOT compensate by changing schema. |
| on | off | Run migrations, but do not automatically initiate `pending` backfills and do not add a backfill readiness gate. Operators run backfills manually while the application safely serves mixed repaired and unrepaired data; already-initiated work remains recoverable. |
| on | on | Run migrations, then required backfills. Keep serving unready until every required automatic backfill validates as `completed`. |

In automatic mode, `pending`, `running`, `paused`, `blocked`, `failed`, and
`validating` required backfills all keep the serving gate closed. A process
restart MUST reconstruct the gate from PostgreSQL, not from whether a local task
was running.

The relay MUST establish the complete set of required automatic backfills for
the deployed application before it evaluates that gate. Late discovery MUST NOT
allow the relay to report ready during an interval in which a required backfill
is unknown to the gate.

In manual mode, incomplete or manually running backfills MUST NOT make the relay
unready. This is safe only because schema/application compatibility is a
mandatory precondition: application reads and writes MUST behave correctly
before, during, and after every associated backfill. A schema change that
requires its data backfill to finish before the new application can safely serve
is non-conforming.

Disabling automatic backfills prevents automatic initiation of `pending`
records; it does not abandon a backfill that an operator already initiated.
Workers MUST recover or take over that durable `running` or `validating` work
after process failure without adding a readiness gate. An operator may pause it
when continued execution is not desired.

Backfill readiness is one input to the relay's serving decision, not a
replacement for database, cache, deletion-fence, shutdown, or schema-safety
checks. Disabling automatic backfills removes only the backfill gate.

## Admin and client contract

Manual mode is conforming only when the full control loop is present. The relay
admin API is the authoritative external boundary, and the supported client UI is
its projection and controller. Neither the client nor an operator CLI may keep a
second lifecycle, synthesize success, or treat cached state as authoritative.

### Read contract

The authorized admin API and client UI MUST expose:

- a list of known definition identities and versions;
- detail for one backfill, including lifecycle state, immutable bound,
  checkpoint-derived progress, current ownership disposition, retry eligibility,
  and timestamps;
- bounded diagnostics for the latest execution or validation failure;
- whether validation has not run, is running, succeeded, or failed; and
- whether the backfill currently participates in the automatic readiness gate.

Progress MUST be presented honestly. Unknown totals, unavailable diagnostics,
and stale or failed reads remain unknown; they MUST NOT be rendered as zero,
complete, healthy, or offline. The server derives status from PostgreSQL on each
read. The client MAY cache for presentation, but a failed refresh cannot turn
cached data into current authority.

### Control contract

The authorized admin API and client UI MUST support:

- `start` for a `pending` backfill;
- `pause` for initiated work in `running` or `validating`;
- `resume` from `paused`; and
- `retry` from `blocked` or `failed` after operator remediation.

Controls express desired lifecycle transitions. They MUST be safe under client
retry, duplicated delivery, concurrent operators, and a response lost after
commit. Repeating the same logical request MUST return the original result or the
already-converged state without duplicating a transition, recapturing a bound,
or starting a second owner. Conflicting requests MUST return a typed conflict and
the current server state so the client can refresh.

`start` MUST durably capture the bound and mark execution initiated before work
is dispatched. In manual mode, that durable intent distinguishes work that may
be recovered from registered `pending` work that automatic configuration MUST
leave untouched.

The normal control surface MUST NOT expose reset, checkpoint rewind, bound
change, record deletion, force-complete, or unfenced claim release. It MUST NOT
expose a generic `cancel`: pause already provides safe interruption while
preserving committed progress. A future destructive recovery mechanism would
require a separate specification, stronger authorization, explicit audit
semantics, and proof that live and stale owners cannot commit across it. A new
definition version is the normal way to supersede incompatible work.

### Authorization and auditability

Backfills are deployment-wide operational state. Read and control operations
MUST require an explicit deployment-operator capability enforced by the relay
admin boundary. Community ownership, channel administration, and ordinary
membership MUST NOT imply backfill authority. Deployments that intentionally
delegate admin authentication to a protected network boundary MUST preserve the
same effective operator restriction and MUST NOT claim individual actor
attribution they do not possess.

Every accepted or rejected control attempt MUST be auditable with the actor or
effective authority source, backfill identity-version, request correlation,
prior and resulting state, claim generation when relevant, time, and a bounded
outcome or diagnostic code. An accepted control mutation and its audit record,
or a durable intent to append that record, MUST commit atomically. An unaudited
state change is not success. Read access to detailed diagnostics SHOULD also be
observable without logging sensitive row data.

Diagnostics returned to clients or written to audit MUST be bounded and
redacted. Raw source rows, credentials, unbounded database errors, and query text
MUST NOT cross the admin boundary.

## Observability

A conforming implementation MUST make operators able to distinguish at least:

- current lifecycle counts and readiness-gating disposition;
- claim acquisition, renewal, loss, takeover, and stale-owner rejection;
- attempted and committed work, checkpoint movement, and lack of progress;
- retryable errors, blocked conditions, exhausted failures, and validation
  outcomes; and
- execution and validation duration.

Signals MUST use bounded label vocabularies. Backfill identifiers, checkpoints,
row values, diagnostics, actor identifiers, and other unbounded or sensitive
values belong in authorized detail views and structured logs, not metric labels.
Missing telemetry is unknown, never implicit success. Metrics and logs MUST be
derivable from or reconcilable with PostgreSQL state and MUST NOT authorize a
transition.

## Conformance

Conformance tests MUST exercise production transaction and admin seams and be
falsifiable: removing the relevant fence, atomic write, authorization check, or
readiness input MUST make a test fail. Test-only lifecycle helpers do not prove
the implementation.

At minimum, the suite covers:

1. **Claim race.** Concurrent workers contend for one eligible backfill; only one
   generation can commit, and eventual takeover preserves a single monotonic
   checkpoint.
2. **Rollback.** A failure after target mutation but before commit leaves both
   target data and checkpoint unchanged; a checkpoint failure also rolls back
   target mutation.
3. **Restart.** Process termination before and after a batch commit, and during
   validation, reconstructs progress solely from PostgreSQL and resumes without
   recapturing the bound.
4. **Stale owner.** Pause, expiry, or takeover occurs while an old worker is in
   flight; its target mutation, checkpoint, and completion attempts are rejected
   at the commit seam.
5. **Validation.** Reaching the bound cannot complete without validation;
   validation failure is durable and repeatable, and only successful validation
   reaches immutable `completed`.
6. **Bounded failure.** Persistent execution error reaches a terminal `failed`
   or `blocked` disposition rather than an unbounded retry loop; operator retry
   preserves bound and checkpoint.
7. **Admin authorization.** Unauthorized reads and controls fail before
   protected state is returned or mutated; community roles alone grant no
   deployment-wide authority.
8. **Admin idempotency.** Duplicate, concurrent, and lost-response control
   requests converge without duplicate starts, generation reuse, or bound
   recapture; conflicts return current state.
9. **Client projection.** The UI renders server/PostgreSQL state, preserves
   unknown and failure states honestly, and sends only the supported controls.
10. **Configuration matrix.** All four schema/backfill control combinations are
    exercised through real startup and readiness paths. Automatic mode gates
    until validation; manual mode never adds the backfill gate; schema safety
    remains independently enforced.

PostgreSQL concurrency tests SHOULD use controlled transaction barriers so the
losing and stale commits are observed deterministically rather than inferred
from timing.

## Optional future formalization

TLA+ is not a prerequisite for this design. The load-bearing obligations today
are at concrete PostgreSQL seams—atomic mutation plus checkpoint, exclusive
claims, generation-fenced commits, takeover, and readiness reconstruction—so
controlled concurrency and transaction tests are required now.

A small state-machine model becomes worthwhile if the lifecycle gains more
ownership modes, takeover paths, or destructive recovery controls. Such a model
would supplement the production-seam tests and would need mutation evidence that
its invariants are non-vacuous; it would not replace database execution tests.

## Implementation correspondence

This section maps the implementation-independent contract to Buzz's intended
component boundaries. Names within a component may change without changing the
specification.

| Specification responsibility | Buzz component correspondence |
|---|---|
| Definition registry, lifecycle engine, PostgreSQL durable store, claim and generation fencing, checkpoint transactions, retry policy, and validation | The new `buzz-backfill` crate owns all backfill concepts and durable behavior. |
| Basic writer-database and transaction access used to make mutation and checkpoint one commit | `buzz-db` supplies narrow database/transaction primitives. It does not acquire backfill records, states, policy, or orchestration, and it does not expose its connection pool. |
| Automatic discovery/execution, the independent configuration controls, startup ordering, and the serving-readiness input | `buzz-relay` composes the backfill engine with existing startup, configuration, and readiness coordination. |
| Authorized list/detail reads, safe lifecycle commands, typed conflicts, audit integration, and bounded diagnostics | The relay's deployment-admin interfaces expose the server contract; operator tooling consumes that contract rather than accessing backfill tables directly. |
| List, detail, progress, diagnostics, validation status, readiness participation, and start/pause/resume/retry controls | The desktop/admin client is a projection and controller over the relay API and PostgreSQL state, never another state store. |

This boundary preserves the repository's focused-crate architecture: the relay
orchestrates, the backfill crate owns its domain and persistence, and the
database crate remains general transaction infrastructure. It also preserves
Buzz's protocol boundary. Backfill administration is an operator HTTP/admin
concern; no Nostr event or NIP is introduced.

## Summary

Buzz data backfills are finite, versioned PostgreSQL state machines. An immutable
upper bound and monotonic typed checkpoint define progress; one transaction binds
each mutation to its checkpoint; and a monotonic claim generation rejects stale
owners after pause, crash, or takeover. Validation, not scan exhaustion, makes
completion durable.

Schema migration and automatic execution remain independent. Automatic mode
gates serving on validated completion; manual mode preserves readiness and is
safe only because the application remains compatible with incomplete data. The
relay admin API and client complete that manual operating loop without becoming
new authorities.
