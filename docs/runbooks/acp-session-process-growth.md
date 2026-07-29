---
title: ACP session process growth recovery
verified: 2026-07-28
review_after: 2026-10-27
topics:
  [
    buzz-acp,
    acp,
    process-growth,
    memory,
    session-close,
    model-switch,
    relay-control,
    recovery,
  ]
references:
  - crates/buzz-acp/src/acp.rs
  - crates/buzz-acp/src/pool.rs
  - crates/buzz-acp/src/lib.rs
  - crates/buzz-acp/src/observer.rs
  - crates/buzz-acp/src/queue.rs
  - crates/buzz-acp/src/relay.rs
  - crates/buzz-acp/tests/startup_signal_cleanup.rs
  - desktop/src/features/agents/lib/liveSwitchOutcome.ts
  - desktop/src/features/agents/ui/ModelPicker.tsx
  - desktop/src/shared/api/agentControl.ts
  - desktop/src/shared/api/types.ts
  - docs/solutions/2026-07-27-acp-session-retirement-leak.md
---

# ACP session process growth

## Trigger

Use this runbook when a long-lived `buzz-acp` process stays responsive but its
descendant count, task count, or memory grows across repeated cancellations or
session rotations.

## Observe without mutation

Identify the supervised service and record:

- service state, main PID, task count, and memory;
- total descendants grouped by executable;
- ACP `session/new`, `session/close`, cancellation, and rotation counts;
- unique channel count compared with session-creation count;
- the installed `buzz-acp` revision and adapter version.

Do not infer a session leak from memory alone. The characteristic signature is
repeated `session/new` for a small, stable channel set without corresponding
retirement, plus retained adapter descendants.

## Contain

Stopping or restarting a production service is an operator-gated action. Before
firing that gate, capture the evidence above and verify that any relay, control,
or alternative-agent service is a separate unit and will remain running.

The live containment gate is exactly:

> `APPROVE BUZZ CONTAINMENT: stop buzz-acp.service only; leave Buzz relay and buzz-acp-codex.service running; verify memory/process recovery; rollback by starting buzz-acp.service.`

This runbook records that gate; it does not fire it. No stop, start, restart, or
other live command is authorized until the operator supplies that exact approval.

For the approved affected unit only:

1. stop the unit;
2. verify its process group and descendants exit;
3. verify host available memory and swap recover;
4. leave unrelated services untouched.

Rollback of containment is to start the same unit, but do not restart the known
leaking revision merely to restore nominal availability unless the operator
accepts the resource-exhaustion risk.

## Remediate

Build and install a reviewed revision that either positively closes a retired
session or replaces the owning adapter process before local state can forget
it. Preserve the previous binary or package as the rollback artifact. Do not
combine the upgrade with credential, relay, channel-membership, or unit
configuration changes.

## Validate

After the operator-approved restart:

1. confirm the running executable and source revision match the reviewed
   artifact;
2. exercise repeated clean cancellation, automatic and owner-requested
   rotation, model switching, and a controlled membership-removal cycle;
3. confirm every retired session produces a successful `session/close`, or a
   capability-negotiated process replacement after the old adapter's direct
   child is reaped and its original process group is proven absent;
4. force separate eager-startup and graceful-shutdown cleanup failures. Eager
   startup must remain visibly degraded without starting an overlapping owner,
   and graceful shutdown must not report a clean exit while ownership of any
   original direct child or process group remains unverified;
5. force one bounded-cleanup failure and confirm the exact slot enters a
   process-lifetime quarantine: maintenance must not refill it after the
   ordinary crash cooldown, and the current supervisor must remain alive in a
   visible degraded state instead of exiting into an automatic restart;
6. force one checked-out task panic and confirm it uses that same quarantine.
   Unwinding loses the typed child owner, so a panic is never sufficient
   evidence for an in-process replacement;
7. confirm a stored numeric process-group ID is only signalled while the live
   direct-child identity still matches its spawn PID. After that identity is
   cleared, the ID may be used for read-only absence probes but never as a
   signal target because the operating system may have reused it;
8. confirm poisoned or still-owning adapters never return to the idle pool;
   terminal dead-lettering clears the batch's `required_agent` pin; exact-slot
   capacity deferral preserves the existing retry budget; cancelled and
   ordinary work share one deterministic global FIFO; and model-switched work
   resumes on its exact slot after replacement. Enqueue the same signed event
   more than once and confirm each occurrence keeps a distinct retry,
   dead-letter, cancelled-provenance, and native-steer identity; one steer
   acknowledgement must release or remove only its exact occurrence, and
   malformed occurrence vectors must fail closed. Give two withheld occurrences
   the same receive timestamp, resolve their acknowledgements in reverse, and
   confirm enqueue FIFO still determines dispatch and reply-anchor order;
9. confirm an accepted owner cancel or rotate never replays its in-flight batch
   when an error, timeout, or task panic wins the completion race;
10. issue two concurrent switches for the same channel and model. Each desktop
    request must generate a distinct 32-character lowercase hexadecimal
    `requestId`; Rust must receive it and echo it byte-for-byte on every
    immediate or asynchronous terminal result; and the desktop must settle a
    request only from its exact ID. Receipt and recycle states remain pending,
    and success means the requested model was applied to the target session.
    Also force a fresh-catalog rejection and confirm the restored prior model
    and its prior request identity are actually applied before the replacement
    session becomes available;
11. verify a command-shaped relay event is owner-authorized before privileged
    classification or replay admission. A control is eligible only for the
    exact active subscription and only when its timestamp strictly postdates
    the timestamp recorded after that subscription's successful `REQ`;
12. fill the observer replay-dedup set with fresh IDs and confirm it retains all
    of them and rejects additional admission until capacity becomes available;
    it must not evict a still-fresh ID and reopen its replay window;
13. verify signature, shape, owner, subscription, and freshness failures are
    rejected before bounded control enqueue. When the valid-control queue is
    full, backpressure must fail closed rather than silently dropping the
    control or mutating replay/routing state. Saturate ordinary telemetry and
    confirm observer control results still drain first from their protected
    FIFO without starving relay reads or pings. Confirm only `accepted=true`
    acknowledges a durable frame, exact rate-limit rejections requeue with
    pacing no earlier than an advertised sixty-second reset, the ninety-second
    confirmation window covers that reset plus positive jitter, one failed
    complete publication gets one bounded ledger replay, terminal denials are
    surfaced without retry spin, and bounded loss remains visible. Fill the
    retained terminal ledger to 1,024 results and verify the newest overflow is
    rejected without evicting older proof and forces failed shutdown
    verification;
14. sample the service cgroup's full descendant count and memory over a duration
   longer than the original reproduction window; process-group proof cannot
   detect a descendant that deliberately escaped with `setsid(2)`;
15. verify completed replacement tasks do not accumulate during sustained relay
   traffic or a quiet interval. Pre-loop and automatic-exit cleanup failures
   must remain alive in non-spawning quarantine until explicit shutdown.
   Graceful shutdown must cooperatively cancel checked-out prompt and heartbeat
   work, recover each typed adapter owner, and perform bounded reap/probe;
   unavailable descendant containment must not be reported as verified;
16. send a second SIGINT or SIGTERM while graceful cleanup is pending and
    confirm it enters the bounded hard-cleanup path. Repeated startup/shutdown
    must leave no orphaned signal-handler tasks. Block `session/new` and the
    configured initial prompt in separate trials; the first must return the
    typed owner for process recycling, and the second must cancel and retire
    the partial session without waiting for the turn timeout;
17. queue at least 48 ordinary observer frames, then emit a terminal control
    result while replay and live chunk coalescing are active. The terminal
    result must interrupt ordinary pacing and arrive inside the desktop
   request timeout. Saturate the protected relay lane separately: already
   admitted terminal proofs must remain in FIFO order, the newest overflow
   must be rejected visibly before any socket write. Submit the same signed
   terminal event through two publisher clones and across disconnected/failed
   replay paths; the duplicate must not queue or write, and the first
   confirmation waiter must remain authoritative. Terminal publication must
   remain pending after the local socket write until the exact relay event
   receives `OK accepted=true`. During graceful shutdown, a missing
   acknowledgement, priority-lane lag, denial, or bounded publisher timeout
   must make shutdown verification fail rather than report clean delivery;
18. block child stdin while cancellation cleanup owes both a pending permission
    response and `session/cancel`; both writes and response drain must remain
    inside one shared grace deadline. Trigger a control immediately after
    `session/new` succeeds but before the configured initial prompt is first
    polled; Buzz must close the partial idle session and must preserve the batch
    without retry/dead-letter accounting while recycling uncertain setup
    ownership;
19. verify unrelated relay and alternative-agent services stayed healthy.

Do not close the runtime incident from a green build or successful restart
alone. Closure requires a bounded post-restart process and memory trend.

If a slot is quarantined, first prove the prior adapter and its full service
cgroup descendant tree are absent. Restarting the reviewed unit is the recovery
action and remains operator-gated; a cooldown, heartbeat, or local PID probe is
not a substitute for that proof.

## Roll back

If the reviewed revision causes a regression:

1. stop only the affected unit;
2. restore the preserved prior artifact;
3. start the unit;
4. verify service health and message flow;
5. continue process and memory monitoring because rollback reintroduces the
   original leak risk.

Record the rollback and keep containment available until a corrected revision
passes the same validation.
