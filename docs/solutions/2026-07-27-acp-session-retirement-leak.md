---
title: Retire ACP sessions before releasing adapter ownership
date: 2026-07-27
category: docs/solutions/incidents
module: buzz-acp
problem_type: incident
component: session-lifecycle
severity: high
applies_when:
  - buzz-acp remains active across repeated cancellation or session rotation
  - an adapter retains per-session subprocesses until explicit teardown
  - local routing state releases a session while its adapter process still owns it
symptoms:
  - buzz-acp stays healthy while its descendant count and memory keep growing
  - repeated session/new calls serve a small stable set of channels
  - retired Claude and MCP subprocess trees remain below the adapter process
root_cause: logic_error
resolution_type: code_fix
related_components:
  - process-lifecycle
  - work-queue
  - model-switch
  - relay-control
tags:
  - buzz-acp
  - acp
  - session-lifecycle
  - process-leak
  - resource-exhaustion
verified: 2026-07-28
review_after: 2026-10-27
topics: [buzz-acp, acp, session-lifecycle, process-leak, resource-exhaustion]
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
  - docs/runbooks/acp-session-process-growth.md
---

# Retired ACP sessions kept adapter subprocess trees alive

## What happened

A long-lived `buzz-acp` deployment accumulated more than 1,800 descendant
processes and more than 70 GB of memory. The service had created 152 ACP
sessions for only two channels, including 150 sessions for one channel, while
its log recorded hundreds of control cancellations.

The adapter process stayed connected and continued accepting new sessions, so
ordinary health checks remained green while retired Claude and MCP subprocess
trees accumulated underneath it.

## Root cause

Buzz removed a session ID from its local routing state after a clean
control-signal cancellation or automatic rotation, but did not send the ACP
`session/close` request first. Local invalidation therefore abandoned the
adapter-owned session rather than retiring it. For adapters that keep a query
and its MCP subprocesses alive until explicit teardown, every later session
created another retained process tree.

## Resolution

The ACP client now negotiates and exposes a bounded `session/close` request.
Buzz sends it only when the adapter advertises the optional capability.
Otherwise Buzz retires ownership by shutting down and replacing the adapter
process group.

The lifecycle now covers:

- clean control cancellation and automatic turn-count/token rotation;
- rotate/model commands that race with natural prompt completion;
- accepted owner cancel/rotate commands whose drop disposition must dominate a
  simultaneous prompt error, timeout, or task panic;
- idle rotate/model commands and channel-membership removal;
- checked-out adapters that return after their channel was removed;
- partial sessions whose Goose system-prompt or initial-message setup fails.

Main-loop-only paths claim the owning adapter and schedule an asynchronous
process recycle rather than deleting local state. Compatibility recycles are
rate-reserved per slot, do not consume crash budget, and preserve live model
intent. Exact-slot queue affinity holds a switched batch until that same slot
has a fresh session; completed replacement tasks are reaped under both busy and
quiet traffic.

Close and cancellation cleanup failures are fail-closed. Buzz preserves
retryable work, emits a redacted cleanup error, and replaces the process group.
Replacement itself is also fail-closed: no new adapter starts unless bounded
shutdown reaps the direct child and proves the original process group absent.
If either proof is unavailable, the exact slot enters a process-lifetime
quarantine that maintenance and the ordinary crash cooldown cannot bypass. The
eager-startup path stays alive in a visible degraded state rather than exiting
into an automatic service restart, and graceful shutdown never reports clean
completion while any original process ownership remains unverified. A
checked-out task panic uses the same quarantine because unwinding destroys the
only typed child owner and therefore cannot produce positive reaping evidence.
An automatic-exit path with any unverified cleanup owner remains alive in that
same non-spawning quarantine until an explicit shutdown request arrives; it
cannot exit into a service-manager restart over a possibly live process group.
Graceful shutdown cooperatively cancels checked-out prompt and heartbeat tasks,
recovers each typed adapter owner, and then performs the bounded adapter
shutdown and absence proof. It does not use task abortion as cleanup evidence.
On platforms where descendant containment is unavailable, cleanup is
unverified and therefore quarantined rather than inferred from direct-child
exit.

The stored spawn-time process-group ID remains useful for read-only absence
probes, but Buzz signals it only while Tokio still exposes the matching live
direct-child PID. Once that identity is cleared, the numeric ID may have been
reused and is never treated as a safe signal target.

Malformed NDJSON and other framing failures stop at the first bad nonempty
frame without logging raw agent output. Quarantined slots retain retryable
queued work and model intent without attempting an overlapping refill; work
that exhausts its retry budget is deliberately dead-lettered and its
`required_agent` exact-slot pin is cleared. Capacity deferral for an exact-slot
batch preserves its existing retry budget rather than resetting it. Cancelled
and ordinary work are ordered through one deterministic global FIFO.
Retry membership is bound to a private UUID created for each enqueue
occurrence, not to the signed Nostr event ID. Separately enqueued copies of the
same signed event therefore remain distinct: a fresh copy cannot inherit an
older occurrence's retry budget, dead-letter fate, or cancelled provenance.
Every batch carries position-aligned occurrence IDs, and missing, partial, or
duplicate identity fails closed instead of being reconstructed from event
content. A process-local enqueue ordinal travels with that identity and breaks
equal-`Instant` ties, so independently resolving native-steer acknowledgements
cannot reverse FIFO order. Native-steer pending state and acknowledgements
carry that same private occurrence ID, so one successful or failed steer
cannot release or remove another enqueue occurrence that happens to share its
signed event ID.

Relay control events are owner-authorized before privileged classification or
replay admission. They must bind to the exact active subscription and strictly
postdate the timestamp recorded after that subscription's successful `REQ`.
Cheap signature, shape, identity, subscription, and freshness validation runs
before bounded enqueue, and a full valid-control queue fails closed instead of
silently dropping a control. Observer replay deduplication retains every fresh
ID; when its bounded set is full it rejects new admission rather than evicting a
still-fresh ID and reopening that replay window.
Observer result delivery reserves a protected control-result FIFO and
acknowledgement window that drain before ordinary telemetry. Terminal model
results therefore bypass telemetry pacing and survive ordinary observer queue
pressure without blocking relay reads or pings. Protected relay saturation
rejects the newest terminal frame before a socket write without evicting an
already-admitted proof. A duplicate signed terminal event is likewise rejected
before queueing or writing, preserving the first publisher's exact confirmation
ownership, and the publisher treats any local priority-lane lag as delivery
uncertainty.
Terminal publication completes only after the exact relay event receives
`OK accepted=true`; a local socket write or queue admission is not proof. An exact transient
rate-limit rejection requeues the same frame with pacing, while terminal
denials are counted and surfaced without an unbounded retry loop. Graceful
shutdown closes the observer source and waits a bounded interval for the
publisher; a rejection, missing relay acknowledgement, lag, join failure, or
timeout fails shutdown verification rather than silently discarding an
accepted model-switch result.

Live model switching now carries one exact request identity end to end. The
desktop generates a distinct 32-character lowercase hexadecimal `requestId` for
each action, Rust receives that same ID, and every immediate or asynchronous
terminal result echoes it byte-for-byte. The desktop accepts a result only for
its exact pending ID, so concurrent requests for the same channel and model
cannot claim each other's result. Receipt and recycle scheduling remain pending;
application-class model rejection is terminal, and success requires
channel-scoped proof that the requested model was applied.
If a fresh adapter catalog rejects a pending model, rollback restores the exact
prior model intent and request identity and applies that restored model to the
fresh session before the session is committed. The rejected model is never
reported as active merely because replacement succeeded.

Automatic failures detected before the main loop enter the same cleanup
quarantine as later ownership failures. Startup signal handlers are removed
when their owner exits, so repeated startup attempts cannot leave orphaned
signal tasks behind. After the first SIGINT or SIGTERM requests graceful
shutdown, both listeners remain active; a repeated signal enters the bounded
hard-cleanup path instead of being swallowed while cleanup is pending.
Control signals also preempt every agent-owning setup await. A preempted
`session/new` is typed as cooperative setup preemption, so its preserved batch
is requeued as cancelled without retry or dead-letter accounting while the
uncertain adapter is recycled. A control that becomes ready after session
creation but before the initial prompt is first polled still closes that
partial idle session, preventing reuse of a session that skipped its configured
initial message. Cancellation cleanup uses one shared grace deadline for
permission-response and `session/cancel` stdin writes as well as response
drain, including when child stdin is backpressured.

## Regression proof

The regression suite drives fake ACP adapters over the real NDJSON transport
and asserts:

1. clean cancellation sends `session/cancel`, drains the prompt response, then
   sends `session/close` before local invalidation;
2. automatic rotation sends `session/close` before the next `session/new`;
3. a rejected close preserves the session ID and queued work and enters
   process-group replacement;
4. representative adapter rejection and timeout close failures emit one
   operator-visible error and never return the poisoned adapter to the idle
   pool;
5. a matching JSON-RPC response without either `result` or `error` is not a
   positive acknowledgement;
6. unadvertised close support triggers bounded process recycling without
   probing the optional method;
7. idle control, membership removal, partial setup, simultaneous-ready model
   switching, panic quarantine, and malformed framing preserve ownership and
   retryable work at their production seams, while retry exhaustion
   dead-letters the batch and clears its `required_agent` exact-slot pin;
8. successfully delivered cancel/rotate controls are recorded by the
   supervisor, so an error-result or panic race drops the owner-discarded batch
   instead of replaying it;
9. malformed NDJSON cannot be skipped in favor of a later matching response,
   and diagnostics contain no raw wire payload;
10. eager startup remains visibly degraded, and graceful shutdown cannot return
    clean success, when direct-child reaping plus original-group absence is
    unverified; task panic quarantines the slot, and a cleared live child
    identity prevents signalling a stored numeric PGID;
11. terminal dead-letter clears `required_agent`, exact-slot capacity deferral
    preserves retry accounting, and interleaved ordinary and cancelled batches
    retain deterministic global FIFO order;
12. owner authorization precedes privileged relay classification; stale,
    pre-subscription, wrong-subscription, misrouted, invalidly signed, and
    structurally invalid controls fail before bounded enqueue; queue saturation
    fails closed; and fresh replay IDs remain protected at dedup capacity;
13. two concurrent switches for the same channel and model carry distinct
    32-character lowercase hexadecimal request IDs through Rust, every
    immediate and asynchronous terminal frame echoes the originating ID, and
    neither desktop request can claim the other's result;
14. separately enqueued copies of one signed event retain distinct occurrence
    identity across ordinary retry, cancelled deferral, capacity eviction,
    active pruning, and native-steer transitions; a steer acknowledgement
    releases or removes only its exact occurrence, and malformed identity
    vectors fail closed; equal receive timestamps retain enqueue FIFO even when
    acknowledgements resolve in reverse order;
15. a fresh-catalog model rejection applies the restored prior model to the
    replacement session and preserves its prior request identity before that
    session becomes available;
16. observer control results retain protected priority FIFO delivery under
    concurrent relay traffic and ordinary telemetry saturation; only
    `accepted=true` acknowledges a frame, exact rate-limit rejections requeue
    with pacing, and terminal denials are visible without retry spin;
17. pre-loop and automatic-exit cleanup failures remain in non-spawning
    quarantine until explicit shutdown; graceful shutdown recovers checked-out
    owners for bounded reap/probe, and unavailable descendant containment
    fails closed;
18. startup signal tasks are removed on shutdown, while a second SIGINT or
    SIGTERM during graceful cleanup enters the hard-cleanup path;
19. shutdown preempts blocked session setup, context preparation, and initial
    prompts instead of waiting for the final prompt boundary; dropped setup
    requests force process recycling, while an in-flight initial prompt is
    cancelled and its partial session retired before the typed owner returns;
20. a live terminal control result interrupts both captured telemetry replay
    pacing and a live coalescer flush, so an ordinary 48-frame backlog cannot
    consume the desktop result timeout before the terminal result publishes;
21. setup preemption preserves an exhausted-budget batch exactly once without
    charging retry budget, a control ready before the initial-prompt future is
    polled closes the newly created idle session, and backpressured cleanup
    writes cannot exceed the shared cancellation grace;
22. protected relay saturation rejects newest without evicting admitted
    terminal proof or writing the rejected frame; duplicate terminal event IDs
    reject before queue/socket side effects without replacing the first
    confirmation waiter; terminal publication remains pending after a local
    socket write until exact relay `OK accepted=true`, and rejection or
    priority-lane loss makes bounded shutdown verification fail closed.

Focused `buzz-acp` lifecycle and desktop request-correlation regressions exercise
these boundaries. Rust formatting and lint gates, desktop unit and typecheck
gates, diff hygiene, and changed-path secret scanning are part of the source
verification. The complete desktop Tauri Clippy leg remains host-blocked by the
missing `gdk-3.0` development package and is not represented as passing.

## Prevention

The regression cases above keep ownership proof, queue identity, model-switch
correlation, and relay admission fail-closed at their production seams. The
linked [ACP session process growth runbook](../runbooks/acp-session-process-growth.md)
defines the separate operator-gated rollout and runtime proof needed to close
the live incident.

## Remaining boundary

This is a source remediation, not proof that a running deployment has adopted
it. Runtime closure requires building and installing the reviewed revision,
restarting only the affected service under its normal operator gate, and
proving that descendant-process and memory counts remain bounded under repeated
cancellation, rotation, model switching, and membership churn. Use the linked
runbook. Process-group verification cannot prove termination of descendants
that deliberately escape with `setsid(2)`, so runtime closure must also inspect
the service cgroup's full descendant and memory trend.
