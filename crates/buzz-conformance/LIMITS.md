# Limits of the runtime conformance gate

The runtime conformance harness is **not a proof.** It says only this:
*for the executions that actually ran with tracing on*, the relay's
ingest/read decisions matched a trace the spec accepts. Coverage is
exactly the set of code paths exercised — no more, no less.

This file says what the gate **doesn't** catch, so reviewers and
operators don't read more into a green run than is there.

## Scope

The harness is wired only at the **ingest/auth/read accept-reject
boundary** in `crates/buzz-relay/src/handlers/{ingest,req,event}.rs`.
That boundary was chosen because:

1. It is where tenant-derived decisions become observable behavior.
2. The spec's `Next` relation is written in those terms.
3. Every other layer (DB filter SQL, Redis pubsub, S3 metadata) is
   downstream of a decision made here.

Decisions made elsewhere — for example, a buggy SQL `WHERE` clause that
silently returns cross-community rows — surface here only if the
projection reads enough of the row to notice. See §"What it does NOT
catch" below.

## Coverage is execution coverage

The gate validates traces from executions you ran. If an unsafe code
path never executes during a CI run, the gate is silent about it. This
is why coverage breach is load-bearing: an entry to a critical seam
that doesn't emit *any* action records `ImplBug`, which fails closed.

But coverage breach can only fire on **paths the harness was armed
on**. If a new endpoint is added that bypasses `EmitGuard::arm`, the
gate is blind. New endpoints touching the tenant boundary MUST arm a
guard at entry — that's enforced by code review, not by the harness.

## What it does NOT catch

- **DB layer leaks the projection doesn't read.** The projection for
  `read_message_rows` and `read_by_id_rows` records a `row_community`
  per returned row. How the emitter computes that label is the design
  question for the held-back req.rs patch — the honest options are
  per-row channel→community lookup, or recording the resolved community
  uniformly (which makes the gate decorative for read confinement).
  The choice is Eva's review call before fixtures land. Until then,
  the read-seam half of the gate is **not yet armed**.

- **Cross-pod leaks.** The harness traces one process. A multi-pod
  leak (NIP-98 replay across pods, fanout to the wrong pod) shows up
  here only on the pod that observes the leak. Cross-pod attacks are
  Sami's adversarial lane, not the conformance gate's.

- **Time-bounded properties.** The spec is untimed; the gate is
  untimed. A bug that only shows up under high concurrency or specific
  ordering is in scope for perf/red-team, not for trace conformance
  (unless it surfaces as an `Inv_NonInterference` violation in the
  trace, which is the only thing the gate watches for).

- **Pubsub fan-out.** Fan-out is **not** a spec action (see the
  docstring in `event.rs`). A leak in fan-out shows up in the
  **receiver's** ingest/read trace, not in the publisher's emit.

- **Type-level fence violations.** `CommunityId` having no `From<Uuid>`
  is enforced by the Rust compiler, not by this gate. If somebody adds
  `From<Uuid>` for `CommunityId`, the production fence is broken and
  this gate won't say so.

- **Spec bugs.** The checker re-implements the spec; if the spec is
  wrong, both pass. Spec correctness is the proof obligation of
  `docs/spec/MultiTenantRelay.tla`, machine-checked by TLC.

## What turning the harness off means

`Tracer = NoopTracer` (the production default) makes every emit and
guard arm a no-op call. The relay still runs and still decides
correctly because the gate is **observation only** — it does not feed
back into the decision. Turning it off only loses observability.

The CI command (below) constructs an in-memory tracer and asserts every
recorded trace against `check_trace`. If you bypass the CI command and
run with `NoopTracer`, you get no signal.

## CI command

The gate's bite is enforced by three test surfaces that MUST stay green
on every PR:

```sh
# 1. Schema + checker unit tests (9 tests). Cover the transition rules
#    directly — every `TraceAction` variant has a passing case and at
#    least one mutation-class bite case.
cargo test -p buzz-conformance --lib

# 2. Replay fixtures (5 tests). Three JSONL traces in
#    crates/buzz-conformance/tests/fixtures/ are committed for reviewer
#    visibility. The test reconstructs each from typed Rust, asserts
#    the committed file matches byte-for-byte (so a schema-change PR
#    must update the fixtures), then replays through `check_trace`:
#
#    - good.jsonl                       → Ok(())
#    - bad_host_channel_mismatch.jsonl  → IllegalTransition
#    - bad_coverage_breach.jsonl        → CoverageBreach
#
#    To intentionally refresh fixtures after a schema bump:
#    BUZZ_CONFORMANCE_UPDATE=1 cargo test -p buzz-conformance --test replay_fixtures
cargo test -p buzz-conformance --test replay_fixtures

# 3. EmitGuard coverage-breach self-test (2 tests in
#    crates/buzz-relay/src/conformance/mod.rs). Proves the Drop guard
#    records `ImplBug` when no emit reaches the tracer, and stays
#    silent when an emit did. The seam-name string flows through.
cargo test -p buzz-relay --lib conformance::

# Together: 9 + 5 + 2 = 16 tests; mutate-bite proven for the NI,
# IllegalTransition, and CoverageBreach gates. The integration replay
# (live relay → JsonlTracer → check_trace) lands with the read-seam
# patch onto Max's req.rs work.
```

The integration replay is the **next** ratchet — once the read-seam
emitter lands on Eva's integration branch the harness will drive the
existing e2e suite with a `JsonlTracer` per request and assert
`check_trace` for every captured trace.
