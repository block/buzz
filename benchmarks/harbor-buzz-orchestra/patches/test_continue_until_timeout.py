#!/usr/bin/env python3
"""Offline tests for the continue_until_timeout phase loop.

The loop decides how long every LHTB trial runs and how its tokens and cost are
totalled, so getting it wrong is expensive in a way that is invisible until the
bill arrives: a merge bug undercounts spend, an exit-condition bug either ends
the trial early (reproducing the very problem the patch exists to fix) or never
ends it at all. Testing it against stubs costs seconds; testing it on a
benchmark box costs hours and real model spend.

Run against a patched harbor:
    PYTHONPATH=<patched-parent> python3 test_continue_until_timeout.py
"""

from __future__ import annotations

import asyncio
import json
import sys
import tempfile
import types
from pathlib import Path

from harbor.models.agent.context import AgentContext
from harbor.trial.errors import AgentTimeoutError
from harbor.trial.single_step import (
    SingleStepTrial,
    _context_from_receipts,
    _format_continue_instruction,
    _merge_agent_contexts,
    _read_verifier_feedback,
    _sum_receipt_cost,
    _verifier_passed,
)

# Each fake phase "spends" this much, split across two seats, so the receipts a
# phase writes total exactly the AgentContext cost that phase reports. That makes
# receipts-vs-agent_result agreement an assertable invariant rather than two
# unrelated fakes -- and disagreement is precisely the bug under test.
PHASE_COST = 1.0
PHASE_SEATS = (("lead-1", 0.6), ("worker-1", 0.4))


class FakeVerifierResult:
    def __init__(self, reward):
        self.rewards = {"reward": reward} if reward is not None else {}


def make_stub(
    *,
    timeout_sec,
    rewards,
    continue_until=True,
    tmp=None,
    raise_after_phase=None,
    unsettled=False,
):
    """A minimal object that the patched _run_agent can be bound to.

    `rewards` is the sequence of interim verifier rewards to hand back, one per
    phase. Running out of them means the verifier keeps failing.
    """
    if tmp is None:
        tmp = Path(tempfile.mkdtemp(prefix="cut-test-"))
    stub = types.SimpleNamespace()
    stub.task = types.SimpleNamespace(
        instruction="BASE INSTRUCTION",
        config=types.SimpleNamespace(
            agent=types.SimpleNamespace(
                continue_until_timeout=continue_until, user=None
            ),
            verifier=types.SimpleNamespace(user=None),
        ),
    )
    stub._agent_timeout_sec = timeout_sec
    stub.result = types.SimpleNamespace(agent_result=None, agent_execution=None)
    stub.tmp = tmp
    stub.paths = types.SimpleNamespace(
        verifier_dir=tmp, artifacts_dir=tmp, agent_dir=tmp / "agent"
    )
    stub.receipts = stub.paths.agent_dir / "buzz" / "receipts.jsonl"
    stub.receipts.parent.mkdir(parents=True, exist_ok=True)
    stub.logger = types.SimpleNamespace(
        info=lambda *a, **k: None, warning=lambda *a, **k: None
    )

    stub.calls = []  # instructions the agent was invoked with
    stub.hide_calls = 0
    stub.synced = 0
    stub.exceptions = []
    stub.raise_after_phase = raise_after_phase
    stub.unsettled = unsettled
    stub._reward_iter = iter(rewards)

    def write_phase_receipts(index):
        """Stand in for BuzzContainerRuntime.run()'s finally.

        The real thing calls bundle.write() -> accounting.write_receipts(), which
        opens this exact path with mode "w". Reproducing the TRUNCATION is the
        whole point: it is what silently discarded every earlier phase.

        `unsettled` reproduces the measured production shape of a CANCELLED phase:
        rows present, every figure zero, because _settle_usage never ran.
        """
        zeroed = stub.unsettled and index == stub.raise_after_phase
        with stub.receipts.open("w", encoding="utf-8") as handle:
            for agent_id, cost in (
                tuple((a, 0.0) for a, _ in PHASE_SEATS) if zeroed else PHASE_SEATS
            ):
                handle.write(
                    json.dumps(
                        {
                            "agent_id": agent_id,
                            "phase_marker": index,
                            "input_tokens": 0 if zeroed else 50,
                            "output_tokens": 0 if zeroed else 5,
                            "cost_usd": cost,
                        },
                        sort_keys=True,
                    )
                    + "\n"
                )

    async def _run_agent_phase(*, target, instruction, timeout_sec, user):
        """Mirror the real helper's ORDERING, which is what the fix turns on.

        harbor's _run_agent_phase assigns a fresh empty AgentContext and a fresh
        TimingInfo, then awaits the agent. BuzzContainerRuntime.run writes the
        receipts in its own finally, so they land whether or not the phase
        finishes. Only on a clean return does BuzzOrchestraAgent.run copy totals
        into the context -- asyncio.wait_for cancels that coroutine on timeout, so
        a timed-out phase leaves the context EMPTY while its receipts are complete.
        """
        stub.calls.append(instruction)
        target.agent_result = AgentContext()
        target.agent_execution = types.SimpleNamespace(
            started_at=f"phase-{len(stub.calls) - 1}-start", finished_at=None
        )
        await asyncio.sleep(0.01)
        write_phase_receipts(len(stub.calls))
        if stub.raise_after_phase == len(stub.calls):
            raise AgentTimeoutError(f"phase {len(stub.calls)} hit the deadline")
        target.agent_result = AgentContext(
            n_input_tokens=100, n_output_tokens=10, cost_usd=PHASE_COST
        )

    async def _run_interim_verifier():
        try:
            return FakeVerifierResult(next(stub._reward_iter))
        except StopIteration:
            return FakeVerifierResult(0.0)

    async def _hide_shared_verifier_tests():
        stub.hide_calls += 1

    async def _sync_agent_output(_target):
        stub.synced += 1

    stub._run_agent_phase = _run_agent_phase
    stub._run_interim_verifier = _run_interim_verifier
    stub._hide_shared_verifier_tests = _hide_shared_verifier_tests
    stub._sync_agent_output = _sync_agent_output
    stub._record_exception = lambda exc: stub.exceptions.append(exc)
    return stub


def run(stub):
    asyncio.run(SingleStepTrial._run_agent(stub))


FAILS = 0


def check(label, cond, detail=""):
    global FAILS
    if cond:
        print(f"  PASS  {label}")
    else:
        FAILS += 1
        print(f"  FAIL  {label}  {detail}")


def test_stops_on_full_pass():
    print("full verifier pass ends the trial early")
    stub = make_stub(timeout_sec=60, rewards=[0.3, 0.7, 1.0])
    run(stub)
    check("ran 3 phases", len(stub.calls) == 3, f"got {len(stub.calls)}")
    check(
        "phase count recorded",
        stub.result.agent_result.metadata.get("continue_until_timeout_phases") == 2,
        str(stub.result.agent_result.metadata),
    )
    check("tests hidden each round", stub.hide_calls == 3, str(stub.hide_calls))
    check("output synced once", stub.synced == 1, str(stub.synced))


def test_first_try_pass_is_single_shot():
    print("passing immediately means exactly one agent run")
    stub = make_stub(timeout_sec=60, rewards=[1.0])
    run(stub)
    check("ran 1 phase", len(stub.calls) == 1, f"got {len(stub.calls)}")
    check("instruction unmodified", stub.calls[0] == "BASE INSTRUCTION")


def test_deadline_stops_the_loop():
    print("a never-passing verifier is bounded by the deadline")
    stub = make_stub(timeout_sec=0.05, rewards=[0.0] * 100)
    run(stub)
    check("loop terminated", 1 <= len(stub.calls) < 100, f"got {len(stub.calls)}")


def test_usage_is_summed_not_clobbered():
    print("per-phase tokens and cost accumulate")
    stub = make_stub(timeout_sec=60, rewards=[0.1, 0.2, 1.0])
    run(stub)
    ar = stub.result.agent_result
    check("input tokens summed", ar.n_input_tokens == 300, str(ar.n_input_tokens))
    check("output tokens summed", ar.n_output_tokens == 30, str(ar.n_output_tokens))
    check("cost summed", abs(ar.cost_usd - 3.0) < 1e-9, str(ar.cost_usd))


def test_agent_execution_start_survives_all_phases():
    """Regression: the real smoke run reported a 3-hour trial as 1.1 seconds.

    _run_agent_phase stamps a fresh TimingInfo per call, so the loop has to put
    the first phase's start back or every duration-derived number (agent-hours,
    $/agent-hour, the latency comparison) silently reports only the last phase.
    """
    print("agent_execution start is preserved across phases")
    stub = make_stub(timeout_sec=60, rewards=[0.1, 0.2, 1.0])
    run(stub)
    check("ran 3 phases", len(stub.calls) == 3, f"got {len(stub.calls)}")
    check(
        "start is phase 0's, not the last phase's",
        stub.result.agent_execution.started_at == "phase-0-start",
        str(stub.result.agent_execution.started_at),
    )


def test_agent_execution_start_survives_a_timed_out_last_phase():
    """Regression: smoke #4 ran 5 phases over 9 minutes and reported 4.5s.

    Spending the whole budget is the *expected* ending for this loop, and the
    phase that hits the deadline stamps its own TimingInfo on the way out — so
    restoring the start only on the success path fixes the rare case and leaves
    the common one broken.
    """
    print("agent_execution start survives an overrun on the last phase")
    stub = make_stub(timeout_sec=60, rewards=[0.1, 0.2])
    real_phase = stub._run_agent_phase

    async def _boom_after_two(*, target, instruction, timeout_sec, user):
        if len(stub.calls) >= 2:
            stub.calls.append(instruction)
            target.agent_execution = types.SimpleNamespace(
                started_at="phase-2-start", finished_at="phase-2-end"
            )
            raise AgentTimeoutError("budget spent")
        await real_phase(
            target=target, instruction=instruction, timeout_sec=timeout_sec, user=user
        )

    stub._run_agent_phase = _boom_after_two
    run(stub)

    check("ran 3 phases", len(stub.calls) == 3, f"got {len(stub.calls)}")
    check("timeout recorded", len(stub.exceptions) == 1, str(stub.exceptions))
    check(
        "start is phase 0's, not the overrunning phase's",
        stub.result.agent_execution.started_at == "phase-0-start",
        str(stub.result.agent_execution.started_at),
    )
    check(
        "usage from the completed phases is kept",
        stub.result.agent_result.n_input_tokens == 200,
        str(stub.result.agent_result.n_input_tokens),
    )


def test_feedback_is_appended():
    print("continuation instructions carry feedback and keep the original task")
    stub = make_stub(timeout_sec=60, rewards=[0.4, 1.0])
    run(stub)
    second = stub.calls[1]
    check("base instruction retained", second.startswith("BASE INSTRUCTION"))
    check("tells the agent not to stop", "Do not stop" in second)
    check("labels the phase", "phase 1" in second)


def test_flag_off_is_single_shot():
    print("continue_until_timeout=false keeps stock behaviour")
    stub = make_stub(timeout_sec=60, rewards=[0.0], continue_until=False)
    run(stub)
    check("ran 1 phase", len(stub.calls) == 1, f"got {len(stub.calls)}")
    check("no interim verification", stub.hide_calls == 0, str(stub.hide_calls))
    check(
        "no phase metadata",
        (stub.result.agent_result.metadata or {}).get("continue_until_timeout_phases")
        is None,
    )


def test_agent_timeout_is_recorded_not_raised():
    print("an agent that overruns the deadline is recorded, not propagated")
    from harbor.trial.errors import AgentTimeoutError

    stub = make_stub(timeout_sec=60, rewards=[0.0, 1.0])

    async def _boom(*, target, instruction, timeout_sec, user):
        stub.calls.append(instruction)
        raise AgentTimeoutError("simulated overrun")

    stub._run_agent_phase = _boom
    run(stub)  # must not raise
    check("exception recorded", len(stub.exceptions) == 1, str(stub.exceptions))
    check("output still synced", stub.synced == 1, str(stub.synced))


def read_rows(path):
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def test_every_phase_receipt_survives():
    """The regression this whole section exists for.

    Before the fix, receipts.jsonl held only the final phase because
    BuzzContainerRuntime.run() rewrites it with mode "w" once per phase. On
    lhtb46-team that reported $1.76 against agent_result's $9.90.
    """
    print("receipts from every phase survive, not just the last")
    stub = make_stub(timeout_sec=60, rewards=[0.3, 0.7, 1.0])
    run(stub)

    kept = sorted(stub.receipts.parent.glob("receipts.phase-*.jsonl"))
    check("one file per phase", len(kept) == 3, [p.name for p in kept])
    check(
        "named in phase order",
        [p.name for p in kept] == [f"receipts.phase-{i:02d}.jsonl" for i in (1, 2, 3)],
        [p.name for p in kept],
    )

    rows = read_rows(stub.receipts)
    check("canonical file holds every row", len(rows) == 6, f"got {len(rows)}")
    check(
        "every phase represented",
        sorted({r["phase_marker"] for r in rows}) == [1, 2, 3],
        str(sorted({r["phase_marker"] for r in rows})),
    )
    check(
        "both seats present in each phase",
        len({(r["agent_id"], r["phase_marker"]) for r in rows}) == 6,
    )

    # The invariant that makes the two accountings interchangeable again.
    receipts_total = sum(r["cost_usd"] for r in rows)
    md = stub.result.agent_result.metadata
    check(
        "receipts total equals agent_result total",
        abs(receipts_total - stub.result.agent_result.cost_usd) < 1e-9,
        f"receipts {receipts_total} vs agent_result "
        f"{stub.result.agent_result.cost_usd}",
    )
    check("phase file count recorded", md.get("receipts_phase_files") == 3, str(md))
    check(
        "recorded receipts cost matches the file",
        abs((md.get("receipts_cost_usd") or -1) - receipts_total) < 1e-9,
        str(md.get("receipts_cost_usd")),
    )


def test_timed_out_phase_receipts_are_kept():
    """The worst case: the phase that spends the budget is the one that raises.

    Its receipts are written by the runtime's own finally *before* the exception
    propagates, so keeping them has to happen on the exception path too --
    otherwise the most expensive phase of the trial is the one that goes missing.
    """
    print("the timed-out final phase keeps its receipts")
    stub = make_stub(timeout_sec=60, rewards=[0.2, 0.4], raise_after_phase=3)
    run(stub)  # must not raise

    check("timeout recorded", len(stub.exceptions) == 1, str(stub.exceptions))
    kept = sorted(stub.receipts.parent.glob("receipts.phase-*.jsonl"))
    check("all three phases kept", len(kept) == 3, [p.name for p in kept])
    rows = read_rows(stub.receipts)
    check("including the phase that timed out", 3 in {r["phase_marker"] for r in rows})

    # The cancelled phase reported nothing, so without recovering it from its own
    # receipts agent_result would be short by exactly one phase -- and it is the
    # last phase of every trial that ends the way this loop expects to end.
    check(
        "agent_result includes the cancelled phase",
        abs(stub.result.agent_result.cost_usd - 3 * PHASE_COST) < 1e-9,
        f"cost_usd {stub.result.agent_result.cost_usd}, expected {3 * PHASE_COST}",
    )
    check(
        "its tokens recovered too",
        stub.result.agent_result.n_input_tokens == 300,
        str(stub.result.agent_result.n_input_tokens),
    )
    check(
        "totals still agree",
        abs(sum(r["cost_usd"] for r in rows) - stub.result.agent_result.cost_usd)
        < 1e-9,
        f"receipts {sum(r['cost_usd'] for r in rows)} vs "
        f"agent_result {stub.result.agent_result.cost_usd}",
    )


def test_unsettled_final_phase_is_flagged_not_zero_merged():
    """The shape actually measured in production, and the trap inside it.

    lhtb46-team/epidemic-inverse-control-audit: 79 phases, AgentTimeoutError,
    agent_result $5.45, receipts $0.00 across 3 rows. _settle_usage runs only on
    BuzzContainerRuntime.run's success path, so a cancelled phase's usage is never
    flushed and is not in either field. Merging that zero would assert the phase
    was free; the trial's cost has to be marked a floor instead.
    """
    print("a cancelled phase priced at zero is flagged, not merged as free")
    stub = make_stub(
        timeout_sec=60, rewards=[0.2, 0.4], raise_after_phase=3, unsettled=True
    )
    run(stub)

    md = stub.result.agent_result.metadata
    check("timeout recorded", len(stub.exceptions) == 1, str(stub.exceptions))
    check(
        "phase still kept on disk",
        len(list(stub.receipts.parent.glob("*phase-*"))) == 3,
    )
    check("the zero phase is flagged", md.get("unsettled_phases") == [3], str(md))
    check(
        "cost counts only the phases that reported",
        abs(stub.result.agent_result.cost_usd - 2 * PHASE_COST) < 1e-9,
        f"got {stub.result.agent_result.cost_usd}, expected {2 * PHASE_COST}",
    )
    check(
        "no phantom zero inflated the token counts",
        stub.result.agent_result.n_input_tokens == 200,
        str(stub.result.agent_result.n_input_tokens),
    )


def test_settled_run_flags_nothing():
    print("a clean multi-phase run flags no unsettled phases")
    stub = make_stub(timeout_sec=60, rewards=[0.3, 0.7, 1.0])
    run(stub)
    check(
        "unsettled_phases empty",
        stub.result.agent_result.metadata.get("unsettled_phases") == [],
        str(stub.result.agent_result.metadata),
    )


def test_single_phase_receipts_are_untouched_in_shape():
    """A one-phase trial must still leave a normal receipts.jsonl.

    Every analysis tool reads receipts.jsonl and sums its rows. If the fix left
    only numbered files behind, those tools would read an absent file as $0 spent
    -- swapping this bug for the worse one it was modelled on.
    """
    print("a single-phase trial still has a normal receipts.jsonl")
    stub = make_stub(timeout_sec=60, rewards=[1.0])
    run(stub)
    check("one phase only", len(stub.calls) == 1, str(len(stub.calls)))
    check("receipts.jsonl exists", stub.receipts.exists())
    rows = read_rows(stub.receipts)
    check("holds that phase's rows", len(rows) == 2, f"got {len(rows)}")
    check(
        "totals agree",
        abs(sum(r["cost_usd"] for r in rows) - stub.result.agent_result.cost_usd)
        < 1e-9,
    )


def test_flag_off_leaves_receipts_alone():
    print("with the flag off, receipts are left exactly as the runtime wrote them")
    stub = make_stub(timeout_sec=60, rewards=[0.0], continue_until=False)
    run(stub)
    check("no phase files created", not list(stub.receipts.parent.glob("*phase-*")))
    check("receipts.jsonl present", stub.receipts.exists())
    check("holds one phase", len(read_rows(stub.receipts)) == 2)


def test_pure_helpers():
    print("helper functions")
    check("passes at 1.0", _verifier_passed(FakeVerifierResult(1.0)))
    check("fails at 0.999", not _verifier_passed(FakeVerifierResult(0.999)))
    check("fails on None", not _verifier_passed(None))
    check("fails on empty rewards", not _verifier_passed(FakeVerifierResult(None)))
    merged = _merge_agent_contexts(None, AgentContext(n_input_tokens=5))
    check("merge with None copies", merged.n_input_tokens == 5)
    both = _merge_agent_contexts(
        AgentContext(n_input_tokens=5), AgentContext(n_input_tokens=7)
    )
    check("merge sums", both.n_input_tokens == 12)
    none_both = _merge_agent_contexts(AgentContext(), AgentContext())
    check("all-None stays None", none_both.n_input_tokens is None)
    txt = _format_continue_instruction("TASK", phase=2, remaining_sec=30, feedback="fb")
    check("feedback embedded", "fb" in txt and "TASK" in txt and "30 seconds" in txt)
    check(
        "missing verifier dir is handled",
        isinstance(_read_verifier_feedback(Path("/nonexistent-xyz")), str),
    )

    # Both receipt readers must fail to None, never to 0.0 / a zeroed context.
    # A zero merges in silently and reads as "this phase was free" -- the exact
    # absent-means-zero mistake the surrounding fix exists to undo.
    check(
        "missing receipts sum to None, not 0.0",
        _sum_receipt_cost(Path("/nonexistent-xyz.jsonl")) is None,
    )
    check(
        "missing receipts give no context, not an empty one",
        _context_from_receipts(Path("/nonexistent-xyz.jsonl")) is None,
    )
    tmp = Path(tempfile.mkdtemp(prefix="cut-helpers-"))
    empty = tmp / "empty.jsonl"
    empty.write_text("", encoding="utf-8")
    check(
        "an empty receipts file gives no context", _context_from_receipts(empty) is None
    )
    good = tmp / "good.jsonl"
    good.write_text(
        "\n".join(
            json.dumps(
                {
                    "agent_id": a,
                    "input_tokens": 10,
                    "output_tokens": 2,
                    "estimated_cache_read_tokens": 7,
                    "cost_usd": c,
                }
            )
            for a, c in (("lead-1", 0.25), ("worker-1", 0.75))
        )
        + "\n",
        encoding="utf-8",
    )
    check("costs sum across seats", abs((_sum_receipt_cost(good) or 0) - 1.0) < 1e-9)
    ctx = _context_from_receipts(good)
    check("context sums input tokens", ctx.n_input_tokens == 20, str(ctx))
    check("context sums output tokens", ctx.n_output_tokens == 4, str(ctx))
    check("context sums cache tokens", ctx.n_cache_tokens == 14, str(ctx))
    check("context sums cost", abs(ctx.cost_usd - 1.0) < 1e-9, str(ctx))


if __name__ == "__main__":
    for fn in (
        test_stops_on_full_pass,
        test_first_try_pass_is_single_shot,
        test_deadline_stops_the_loop,
        test_usage_is_summed_not_clobbered,
        test_agent_execution_start_survives_all_phases,
        test_agent_execution_start_survives_a_timed_out_last_phase,
        test_feedback_is_appended,
        test_flag_off_is_single_shot,
        test_agent_timeout_is_recorded_not_raised,
        test_every_phase_receipt_survives,
        test_timed_out_phase_receipts_are_kept,
        test_unsettled_final_phase_is_flagged_not_zero_merged,
        test_settled_run_flags_nothing,
        test_single_phase_receipts_are_untouched_in_shape,
        test_flag_off_leaves_receipts_alone,
        test_pure_helpers,
    ):
        fn()
    print()
    print("FAILURES:", FAILS)
    sys.exit(1 if FAILS else 0)
