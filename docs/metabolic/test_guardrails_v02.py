#!/usr/bin/env python3
import time
from guardrails_v02 import (
    AdapterCaps,
    FailureReason,
    GuardState,
    WakeBudget,
    admit_wake,
    monitor_failure,
    new_turn,
)

def base_wake(**kw):
    w = {
        "schema": "metabolic.wake.v0",
        "event_id": "a" * 64,
        "channel_id": "92297894-c2e8-4df1-a710-d1cfd1032d5e",
        "t": "team.v0.task.completed",
        "urgency": "P0",
        "seat_id": "home-grok",
        "pubkey": "ce3a" + "0" * 60,
        "summary": "unblocked: meta-auth",
        "task_id": "meta-auth",
        "correlation_id": "corr-1",
    }
    w.update(kw)
    return w

def main():
    budget = WakeBudget(max_events_per_turn=2, max_context_bytes=100, per_task_cooldown_secs=10)
    caps = AdapterCaps(max_context_bytes=100)
    st = GuardState()
    t0 = 1_000_000.0

    r = admit_wake(st, base_wake(), budget, caps, now=t0)
    assert r["action"] == "AdmitCortex", r
    r2 = admit_wake(st, base_wake(), budget, caps, now=t0 + 1)
    assert r2["action"] == "suppress" and r2["reason"] == "replay", r2
    # second event same turn
    r3 = admit_wake(st, base_wake(event_id="b" * 64, correlation_id="corr-2", task_id="other"), budget, caps, now=t0 + 1)
    assert r3["action"] == "AdmitCortex", r3
    # overflow 3rd
    r4 = admit_wake(st, base_wake(event_id="c" * 64, correlation_id="corr-3", task_id="t3"), budget, caps, now=t0 + 1)
    assert r4["action"] == "overflow", r4
    new_turn(st)
    # cooldown same task
    r5 = admit_wake(st, base_wake(event_id="d" * 64, correlation_id="corr-4"), budget, caps, now=t0 + 5)
    assert r5["action"] == "suppress" and r5["reason"] == "cooldown", r5
    r6 = admit_wake(st, base_wake(event_id="e" * 64, correlation_id="corr-5"), budget, caps, now=t0 + 15)
    assert r6["action"] == "AdmitCortex", r6
    # schema
    r7 = admit_wake(st, base_wake(event_id="f" * 64, summary=""), budget, caps, now=t0 + 20)
    assert r7["action"] == "diagnostic" and r7["reason"] == "schema", r7
    # idempotent correlation
    r8 = admit_wake(st, base_wake(event_id="1" * 64, correlation_id="corr-5", task_id="t9"), budget, caps, now=t0 + 50)
    assert r8["action"] == "suppress" and r8["reason"] == "idempotent", r8
    mf = monitor_failure(FailureReason.STALE_NERVE, "unit down")
    assert mf["reason"] == "stale_nerve"
    print("ALL_V02_GUARDRAIL_TESTS_OK")

if __name__ == "__main__":
    main()
