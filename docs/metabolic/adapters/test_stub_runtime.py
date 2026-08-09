#!/usr/bin/env python3
"""Third-runtime adapter stub tests — zero network, zero LLM."""
from __future__ import annotations

import json
import os
import sys
import tempfile
from pathlib import Path

_HERE = Path(__file__).resolve().parent
_METABOLIC = _HERE.parent
sys.path.insert(0, str(_HERE))
sys.path.insert(0, str(_METABOLIC))

import stub_runtime as stub  # noqa: E402
from guardrails_v02 import Action  # noqa: E402


def test_arm_status_disarm():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        s = stub.arm("local-llm", "t1", room="room-uuid", room_name="demo", transport="poll")
        assert s.runtime == "local-llm"
        assert Path(tmp, "local-llm", "t1", "session.json").exists()
        st = stub.status("local-llm", "t1")
        assert st["nerve"] == "attached"
        assert st["pending"] == 0
        h = stub.health("local-llm", "t1", stale_after_secs=9999)
        assert h.get("health") == "poll"
        stub.disarm("local-llm", "t1")
        assert stub.load_session("local-llm", "t1") is None


def test_antigravity_alias():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        s = stub.arm("antigravity", "agy", room="r1")
        assert s.runtime == "local-llm"  # alias
        assert Path(tmp, "local-llm", "agy", "session.json").exists()


def test_on_wake_admit_and_transport_replay():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        session = stub.arm("local-llm", "t2", room="ch-1")
        wake = stub.make_demo_wake(1, session, summary="hello third runtime")
        d1 = stub.on_wake(session, wake, start_turn=True, now=1000.0)
        assert d1["action"] == Action.ADMIT.value, d1
        assert "cortex" in d1
        assert d1["cortex"]["summary"].startswith("hello")
        assert "content" not in d1["cortex"]  # W1.1 summary+ids default
        session = stub.load_session("local-llm", "t2")
        d2 = stub.on_wake(session, wake, start_turn=False, now=1001.0)
        assert d2["action"] == Action.IGNORE.value
        assert d2["reason"] == "transport_replay"


def test_overflow_batch():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_MAX_EVENTS"] = "3"
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        session = stub.arm("local-llm", "t3", room="ch-2")
        wakes = [stub.make_demo_wake(i, session) for i in range(4)]
        results = stub.inject_many("local-llm", "t3", wakes, single_turn=True)
        admits = [r for r in results if r.get("action") == Action.ADMIT.value]
        overflows = [r for r in results if r.get("action") == Action.OVERFLOW.value]
        assert len(admits) == 3, results
        assert len(overflows) == 1, results
        session = stub.load_session("local-llm", "t3")
        assert len(session.pending) >= 1  # overflow left pending


def test_schema_diagnostic():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        session = stub.arm("local-llm", "t4", room="ch-3")
        bad = stub.make_demo_wake(9, session, summary="")
        # force empty summary after normalize still has (empty) — use missing event_id
        bad["event_id"] = ""
        bad["summary"] = "x"
        d = stub.on_wake(session, bad, start_turn=True, now=50.0)
        assert d["action"] == Action.DIAGNOSTIC.value, d


def test_cli_demo_overflow(capsys_disabled=True):
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        rc = stub.main(
            [
                "demo-overflow",
                "--runtime",
                "local-llm",
                "--seat",
                "cli-demo",
                "--room",
                "00000000-0000-0000-0000-000000000001",
            ]
        )
        assert rc == 0, rc


def test_jsonl_fact_to_wake_and_watch_file():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        room = "92297894-c2e8-4df1-a710-d1cfd1032d5e"
        session = stub.arm(
            "local-llm",
            "watch-t",
            room=room,
            room_name="agent-metabolism",
            self_pubkey="selfpk" + "0" * 58,
            since=1_000_000,
        )
        # self suppress
        self_fact = {
            "id": "s" * 64,
            "created_at": 1_000_001,
            "channel_id": room,
            "pubkey": session.self_pubkey,
            "content": "hello from me",
            "kind": 9,
            "tags": [["h", room]],
        }
        assert stub.jsonl_fact_to_wake(self_fact, session) is None
        # empty suppress
        empty = {
            "id": "e" * 64,
            "created_at": 1_000_002,
            "channel_id": room,
            "pubkey": "ab" + "0" * 62,
            "content": "",
            "kind": 9,
            "tags": [],
        }
        assert stub.jsonl_fact_to_wake(empty, session) is None
        # good fact
        good = {
            "id": "g" * 64,
            "created_at": 1_000_003,
            "channel_id": room,
            "pubkey": "cd" + "0" * 62,
            "content": "team.v0.task.completed task_id=meta-auth unblocked: meta-auth",
            "kind": 9,
            "tags": [["h", room]],
        }
        wake = stub.jsonl_fact_to_wake(good, session)
        assert wake is not None
        assert wake["event_id"] == "g" * 64
        assert wake["t"] == "team.v0.task.completed"
        assert wake["task_id"] == "meta-auth"
        assert wake["urgency"] in ("P0", "P1")

        # JSONL file → watch path
        path = Path(tmp) / "facts.jsonl"
        facts = [
            good,
            {
                "id": "h" * 64,
                "created_at": 1_000_004,
                "channel_id": room,
                "pubkey": "ef" + "0" * 62,
                "content": "plain co-lab note",
                "kind": 9,
                "tags": [],
            },
            self_fact,
        ]
        path.write_text("\n".join(json.dumps(f) for f in facts) + "\n")
        rc = stub.main(
            [
                "watch",
                "--runtime",
                "local-llm",
                "--seat",
                "watch-t",
                "--file",
                str(path),
            ]
        )
        assert rc == 0, rc
        session = stub.load_session("local-llm", "watch-t")
        assert session is not None
        assert session.since >= 1_000_004
        # two admits (self filtered); stream mode = new turn each
        assert len(session.admitted_log) == 2, session.admitted_log


def test_watch_shared_turn_overflow():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_MAX_EVENTS"] = "2"
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        room = "11111111-1111-1111-1111-111111111111"
        stub.arm("local-llm", "batch-w", room=room, since=1)
        lines = []
        for i in range(4):
            lines.append(
                json.dumps(
                    {
                        "id": f"{i:02d}" + ("b" * 62),
                        "created_at": 10 + i,
                        "channel_id": room,
                        "pubkey": "aa" + "0" * 62,
                        "content": f"burst {i}",
                        "kind": 9,
                        "tags": [],
                    }
                )
            )
        path = Path(tmp) / "burst.jsonl"
        path.write_text("\n".join(lines) + "\n")
        # capture via process_jsonl_stream shared turn
        session = stub.load_session("local-llm", "batch-w")
        results = stub.process_jsonl_stream(session, lines, shared_turn=True)
        admits = [r for r in results if r.get("action") == Action.ADMIT.value]
        overflows = [r for r in results if r.get("action") == Action.OVERFLOW.value]
        assert len(admits) == 2, results
        assert len(overflows) >= 1, results


def test_product_driver_hooks():
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        os.environ["BUZZ_DRIVER_DRY_RUN"] = "1"
        # local-llm dry_run draft on AdmitCortex
        session = stub.arm(
            "local-llm",
            "drv-llm",
            room="room-d1",
            driver="local-llm",
        )
        assert session.driver == "local-llm"
        d = stub.on_wake(
            session,
            stub.make_demo_wake(1, session, summary="driver hook probe"),
            start_turn=True,
            now=50.0,
        )
        assert d["action"] == Action.ADMIT.value
        assert d.get("driver") is not None, d
        assert d["driver"]["driver"] == "local-llm"
        assert d["driver"]["status"] == "dry_run"
        assert d["driver"]["draft"]
        session = stub.load_session("local-llm", "drv-llm")
        assert len(session.driver_log) == 1

        # notify driver
        session2 = stub.arm("local-llm", "drv-n", room="room-d2", driver="notify")
        d2 = stub.on_wake(
            session2,
            stub.make_demo_wake(2, session2, summary="notify me"),
            start_turn=True,
            now=60.0,
        )
        assert d2["driver"]["driver"] == "notify"
        assert d2["driver"]["action"] == "notify"

        # antigravity stub surface
        session3 = stub.arm("local-llm", "drv-agy", room="room-d3", driver="antigravity")
        d3 = stub.on_wake(
            session3,
            stub.make_demo_wake(3, session3, summary="agy hook"),
            start_turn=True,
            now=70.0,
        )
        assert d3["driver"]["driver"] == "antigravity"
        assert d3["driver"]["status"] == "not_implemented"

        # none = no product sink
        session4 = stub.arm("local-llm", "drv-0", room="room-d4", driver="none")
        d4 = stub.on_wake(
            session4,
            stub.make_demo_wake(4, session4, summary="stdout only"),
            start_turn=True,
            now=80.0,
        )
        assert d4.get("driver") is None

        rc = stub.main(["drivers"])
        assert rc == 0


def test_local_llm_cmd_resolution_and_real():
    from drivers.local_llm import LocalLlmDriver, ollama_reachable, resolve_local_llm_cmd
    from drivers.base import DriverContext

    cmd = resolve_local_llm_cmd()
    assert "run_local_llm.py" in cmd, cmd

    # dry_run still offline even when cmd exists
    with tempfile.TemporaryDirectory() as tmp:
        os.environ["BUZZ_ADAPTER_STATE_DIR"] = tmp
        os.environ["BUZZ_ADMIT_COOLDOWN_SECS"] = "0"
        os.environ["BUZZ_DRIVER_DRY_RUN"] = "1"
        session = stub.arm("local-llm", "llm-dry", room="r-llm", driver="local-llm")
        d = stub.on_wake(
            session,
            stub.make_demo_wake(7, session, summary="dry path"),
            start_turn=True,
            now=90.0,
        )
        assert d["driver"]["status"] == "dry_run", d["driver"]

    if not ollama_reachable():
        print("SKIP real local-llm (ollama not reachable)")
        return

    # Real ollama path (bounded)
    os.environ["BUZZ_DRIVER_DRY_RUN"] = "0"
    os.environ.setdefault("BUZZ_DRIVER_LOCAL_LLM_MODEL", "gemma3:4b")
    os.environ.setdefault("BUZZ_DRIVER_LOCAL_LLM_NUM_PREDICT", "40")
    os.environ.setdefault("BUZZ_DRIVER_LOCAL_LLM_TIMEOUT", "120")
    cortex = {
        "schema": "metabolic.wake.v0",
        "event_id": "c" * 64,
        "t": "team.v0.room.message",
        "urgency": "P2",
        "task_id": None,
        "correlation_id": None,
        "summary": "Say hello in five words or fewer.",
    }
    ctx = DriverContext(
        runtime="local-llm",
        seat="llm-real",
        room="room-x",
        room_name="test",
        dry_run=False,
        hitl=True,
        allow_reply=False,
    )
    result = LocalLlmDriver().handle_admit(cortex, ctx)
    assert result.status == "ok", result
    assert result.draft and result.draft != "NO_REPLY" or result.draft == "NO_REPLY"
    assert len(result.draft) <= 800
    print(f"REAL_LOCAL_LLM_OK draft={result.draft[:120]!r}")


def main():
    test_arm_status_disarm()
    test_antigravity_alias()
    test_on_wake_admit_and_transport_replay()
    test_overflow_batch()
    test_schema_diagnostic()
    test_cli_demo_overflow()
    test_jsonl_fact_to_wake_and_watch_file()
    test_watch_shared_turn_overflow()
    test_product_driver_hooks()
    test_local_llm_cmd_resolution_and_real()
    print("ALL_THIRD_RUNTIME_STUB_TESTS_OK")


if __name__ == "__main__":
    main()
