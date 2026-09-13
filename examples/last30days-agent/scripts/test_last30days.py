#!/usr/bin/env python3
"""Offline regressions for last30days-agent swarm (no live network calls).

Run from the pack root or this directory:
  python3 scripts/test_last30days.py
  python3 test_last30days.py
"""

from __future__ import annotations

import json
import os
import re
import stat
import sys
import tempfile
import time
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import last30days as sw  # noqa: E402

HEX64_A = "a" * 64
HEX64_B = "b" * 64
HEX64_C = "c" * 64
UUID_A = "11111111-2222-4333-8444-555555555555"
FAKE_KEY = "sk-or-v1-TESTONLY-not-a-real-key-xxxxxxxxxxxxxxxx"


def _ok_body(content: str, *, finish: str = "stop", completion: int = 500) -> dict[str, Any]:
    return {
        "id": "gen-test",
        "provider": "TestProvider",
        "model": sw.configured_model(),
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": completion,
            "total_tokens": 100 + completion,
            "cost": 0.001,
        },
        "choices": [
            {
                "finish_reason": finish,
                "message": {"role": "assistant", "content": content},
            }
        ],
    }


class _FakeResp:
    def __init__(self, body: dict[str, Any]):
        self._raw = json.dumps(body).encode("utf-8")

    def read(self) -> bytes:
        return self._raw

    def __enter__(self) -> "_FakeResp":
        return self

    def __exit__(self, *args: Any) -> None:
        return None


class TestIdentityValidation(unittest.TestCase):
    def test_valid_identity_ok(self) -> None:
        sw.validate_shared_identity(HEX64_A, HEX64_B, UUID_A)

    def test_missing_event_id(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.validate_shared_identity(None, HEX64_B, UUID_A)
        self.assertIn("event-id", str(cm.exception).lower())

    def test_short_event_id(self) -> None:
        with self.assertRaises(RuntimeError):
            sw.validate_shared_identity("abc", HEX64_B, UUID_A)

    def test_bad_requester(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.validate_shared_identity(HEX64_A, "not-hex", UUID_A)
        self.assertIn("requester", str(cm.exception).lower())

    def test_bad_channel_uuid(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.validate_shared_identity(HEX64_A, HEX64_B, "not-a-uuid")
        self.assertIn("channel", str(cm.exception).lower())


class TestSharedEvidenceRefusal(unittest.TestCase):
    def test_skip_evidence_refused(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.validate_shared_evidence_mode(skip_evidence=True, evidence_file=None)
        self.assertIn("skip-evidence", str(cm.exception))

    def test_evidence_file_refused(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.validate_shared_evidence_mode(
                skip_evidence=False, evidence_file=Path("/tmp/fake.md")
            )
        self.assertIn("evidence-file", str(cm.exception))

    def test_normal_ok(self) -> None:
        sw.validate_shared_evidence_mode(skip_evidence=False, evidence_file=None)


class TestTopicNormalizeCap(unittest.TestCase):
    def test_owner_mode_no_cap(self) -> None:
        huge = "x" * 2000
        self.assertEqual(sw.normalize_topic(huge, enforce_gates=False), huge)

    def test_control_chars_stripped_under_enforce(self) -> None:
        out = sw.normalize_topic("a\x00b\x1fc\rd\ne\tf", enforce_gates=True)
        self.assertEqual(out, "abcdef")

    def test_cap_truncates_at_max(self) -> None:
        self.assertEqual(sw.MAX_TOPIC_CHARS, 500)
        huge = "y" * 5000
        out = sw.normalize_topic(huge, enforce_gates=True)
        self.assertEqual(len(out), 500)
        self.assertEqual(out, "y" * 500)

    def test_empty_after_normalize_rejected(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.normalize_topic("\x00\x01\x02", enforce_gates=True)
        self.assertIn("empty topic", str(cm.exception).lower())

    def test_run_swarm_caps_before_evidence_and_model(self) -> None:
        def boom_gather(topic: str, **kwargs: Any) -> Path:
            raise AssertionError("should not gather with uncapped topic")

        with tempfile.TemporaryDirectory() as td:
            state = Path(td) / "state"
            gates = Path(td) / "gates"
            with mock.patch.object(sw, "STATE_ROOT", state):
                with mock.patch.object(sw, "GATES_ROOT", gates):
                    with mock.patch.object(sw, "gather_evidence", side_effect=boom_gather):
                        with mock.patch.object(
                            sw,
                            "chat_completions_retry",
                            side_effect=AssertionError("no model"),
                        ):
                            with mock.patch.dict(
                                os.environ, {"OPENAI_API_KEY": FAKE_KEY}
                            ):
                                result = sw.run_swarm(
                                    "Z" * 8000,
                                    event_id=HEX64_A,
                                    requester=HEX64_B,
                                    channel=UUID_A,
                                    enforce_gates=True,
                                )
        self.assertEqual(len(result.topic), 500)
        self.assertEqual(result.topic, "Z" * 500)
        self.assertEqual(sw.RESERVE_USD, 0.5)

    def test_cost_bound_prompt_uses_capped_topic(self) -> None:
        capped = sw.normalize_topic("q" * 10000, enforce_gates=True)
        prompt = sw.worker_prompt(capped, "product_surface", "desc", "evidence body")
        self.assertIn(f"Topic: {capped}", prompt)
        self.assertNotIn("q" * 501, prompt)
        synth = sw.synthesis_prompt(capped, [("product_surface", "body")], 10)
        self.assertIn(f"Topic: {capped}", synth)
        self.assertLessEqual(len(capped), sw.MAX_TOPIC_CHARS)


class TestNarrowKeyRead(unittest.TestCase):
    def test_env_key_no_mutation(self) -> None:
        with mock.patch.dict(os.environ, {"OPENAI_API_KEY": FAKE_KEY}, clear=False):
            before = dict(os.environ)
            key = sw._api_key()
            after = dict(os.environ)
            self.assertEqual(key, FAKE_KEY)
            self.assertEqual(before, after)

    def test_prefers_last30days_key(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "LAST30DAYS_API_KEY": "l30d-key-value-xxxxxxxx",
                "OPENAI_API_KEY": "openai-key-value-xxxxxxxx",
            },
            clear=False,
        ):
            self.assertEqual(sw._api_key(), "l30d-key-value-xxxxxxxx")

    def test_missing_key_raises(self) -> None:
        clean = {
            k: v
            for k, v in os.environ.items()
            if k
            not in (
                "LAST30DAYS_API_KEY",
                "OPENAI_API_KEY",
                "OPENROUTER_API_KEY",
            )
        }
        with mock.patch.dict(os.environ, clean, clear=True):
            with self.assertRaises(RuntimeError) as cm:
                sw._api_key()
            self.assertIn("API key not set", str(cm.exception))
            # Public errors must not point at host-local secret stores.
            self.assertNotRegex(str(cm.exception).lower(), r"env[-_]?file|\.env")


class TestChildEnvScrub(unittest.TestCase):
    def test_exact_allowlist_only(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "PATH": "/usr/bin",
                "HOME": "/tmp/home",
                "USER": "adopter",
                "LAST30DAYS_EVIDENCE_CMD": "echo hi",
                "LAST30DAYS_EVIDENCE_TIMEOUT": "60",
                "LAST30DAYS_API_KEY": "should-not-pass",
                "OPENAI_API_KEY": FAKE_KEY,
                "RANDOM_SECRET": "nope",
            },
            clear=False,
        ):
            env = sw._scrubbed_child_env(FAKE_KEY)
        self.assertEqual(env.get("PATH"), "/usr/bin")
        self.assertEqual(env.get("LAST30DAYS_EVIDENCE_CMD"), "echo hi")
        self.assertNotIn("LAST30DAYS_API_KEY", env)
        self.assertNotIn("OPENAI_API_KEY", env)
        self.assertNotIn("RANDOM_SECRET", env)

    def test_key_in_value_dropped(self) -> None:
        with mock.patch.dict(
            os.environ,
            {"PATH": f"/usr/bin:{FAKE_KEY}", "HOME": "/h"},
            clear=False,
        ):
            env = sw._scrubbed_child_env(FAKE_KEY)
        self.assertNotIn("PATH", env)


class TestOwnerOnlyModes(unittest.TestCase):
    def test_mkdir_and_write_modes(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "private"
            sw._mkdir_private(root)
            mode_dir = stat.S_IMODE(root.stat().st_mode)
            self.assertEqual(mode_dir, 0o700)
            f = root / "x.txt"
            sw._write_private(f, "hello\n")
            mode_f = stat.S_IMODE(f.stat().st_mode)
            self.assertEqual(mode_f, 0o600)


class TestContentUsability(unittest.TestCase):
    def test_reasoning_only_fails_no_fallback(self) -> None:
        body = {
            "id": "g1",
            "provider": "P",
            "model": sw.configured_model(),
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 900,
                "total_tokens": 910,
                "cost": 0.01,
            },
            "choices": [
                {
                    "finish_reason": "length",
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "reasoning": "x" * 900,
                    },
                }
            ],
        }
        with mock.patch("urllib.request.urlopen", return_value=_FakeResp(body)):
            text, receipt = sw.chat_completions(
                key=FAKE_KEY,
                model=sw.configured_model(),
                prompt="hi",
                role="worker:test",
                max_tokens=900,
                min_chars=200,
            )
        self.assertEqual(text, "")
        self.assertFalse(receipt.ok)
        self.assertIn("empty message.content", receipt.error or "")

    def test_short_content_fails(self) -> None:
        body = _ok_body("short", completion=50)
        with mock.patch("urllib.request.urlopen", return_value=_FakeResp(body)):
            text, receipt = sw.chat_completions(
                key=FAKE_KEY,
                model=sw.configured_model(),
                prompt="hi",
                role="worker:test",
                max_tokens=4000,
                min_chars=200,
            )
        self.assertFalse(receipt.ok)
        self.assertIn("too short", receipt.error or "")

    def test_retry_then_fail(self) -> None:
        body = _ok_body("", finish="length", completion=1400)
        body["choices"][0]["message"]["reasoning"] = "hidden" * 50
        calls = {"n": 0}

        def _urlopen(*a: Any, **k: Any) -> _FakeResp:
            calls["n"] += 1
            return _FakeResp(body)

        with mock.patch("urllib.request.urlopen", side_effect=_urlopen):
            with mock.patch.object(sw.time, "sleep", return_value=None):
                text, receipts = sw.chat_completions_retry(
                    key=FAKE_KEY,
                    model=sw.configured_model(),
                    prompt="hi",
                    role="worker:test",
                    max_tokens=4000,
                    min_chars=200,
                    max_attempts=3,
                )
        self.assertEqual(text, "")
        self.assertEqual(len(receipts), 3)
        self.assertTrue(all(not r.ok for r in receipts))
        self.assertEqual(calls["n"], 3)

    def test_min_success_is_ten(self) -> None:
        self.assertEqual(sw.DEFAULT_MIN_SUCCESS, 10)
        self.assertEqual(sw.DEFAULT_WORKERS, 10)
        self.assertEqual(sw.MIN_SUCCESS, 10)
        self.assertEqual(sw.WORKER_COUNT, 10)

    def test_default_model_is_deepseek_v4_pro(self) -> None:
        self.assertEqual(sw.DEFAULT_MODEL, "deepseek/deepseek-v4-pro")


def _seed_gate_state(
    *,
    spend: dict[str, Any] | None = None,
    requesters: dict[str, Any] | None = None,
    idempotency: dict[str, Any] | None = None,
    day: str | None = None,
) -> Path:
    """Write a consolidated gate-state.json for tests (single-file schema)."""
    paths = sw._gates_paths()
    state = sw._empty_gate_state()
    if idempotency is not None:
        state["idempotency"] = dict(idempotency)
    d = day or sw._utc_day()
    bucket: dict[str, Any] = {
        "requesters": dict(requesters or {}),
        "spend": dict(spend or {"total_usd": 0.0, "reserved_usd": 0.0}),
    }
    state["by_day"][d] = bucket
    sw._atomic_save_gate_state(state, paths["state"])
    return paths["state"]


def _gate_spend(state: dict[str, Any] | None = None) -> dict[str, Any]:
    st = state if state is not None else sw._load_gate_state()
    return dict(sw._day_bucket(st).get("spend") or {})


def _gate_requesters(state: dict[str, Any] | None = None) -> dict[str, Any]:
    st = state if state is not None else sw._load_gate_state()
    return dict(sw._day_bucket(st).get("requesters") or {})


def _gate_idemp(state: dict[str, Any] | None = None) -> dict[str, Any]:
    st = state if state is not None else sw._load_gate_state()
    return dict(st.get("idempotency") or {})


class TestSpendReservation(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.gates = Path(self._td.name)
        self._patchers = [
            mock.patch.object(sw, "GATES_ROOT", self.gates),
            mock.patch.object(sw, "GLOBAL_DAILY_SPEND_USD", 1.0),
            mock.patch.object(sw, "RESERVE_USD", 0.5),
            mock.patch.object(sw, "COOLDOWN_S", 0),
            mock.patch.object(sw, "DAILY_QUOTA", 100),
        ]
        for p in self._patchers:
            p.start()

    def tearDown(self) -> None:
        for p in self._patchers:
            p.stop()
        self._td.cleanup()

    def test_reserve_blocks_when_spent_plus_reserve_exceeds(self) -> None:
        _seed_gate_state(spend={"total_usd": 0.6, "reserved_usd": 0.0})
        with self.assertRaises(RuntimeError) as cm:
            sw.check_and_reserve_gates(
                event_id=HEX64_A,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )
        self.assertIn("reservation denied", str(cm.exception))

    def test_reserve_allows_when_room(self) -> None:
        _seed_gate_state(spend={"total_usd": 0.2, "reserved_usd": 0.0})
        meta = sw.check_and_reserve_gates(
            event_id=HEX64_A,
            requester=HEX64_B,
            channel=UUID_A,
            reserve_usd=0.5,
        )
        self.assertEqual(meta["spend_reserved_this_run"], 0.5)
        spend = _gate_spend()
        self.assertAlmostEqual(spend["reserved_usd"], 0.5)

    def test_ceiling_not_only_spent_gte(self) -> None:
        _seed_gate_state(spend={"total_usd": 0.9, "reserved_usd": 0.0})
        with self.assertRaises(RuntimeError):
            sw.check_and_reserve_gates(
                event_id=HEX64_C,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )


class TestIdempotencyAndLockOrder(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.gates = Path(self._td.name)
        self._patchers = [
            mock.patch.object(sw, "GATES_ROOT", self.gates),
            mock.patch.object(sw, "GLOBAL_DAILY_SPEND_USD", 10.0),
            mock.patch.object(sw, "RESERVE_USD", 0.1),
            mock.patch.object(sw, "COOLDOWN_S", 0),
            mock.patch.object(sw, "DAILY_QUOTA", 100),
        ]
        for p in self._patchers:
            p.start()

    def tearDown(self) -> None:
        for p in self._patchers:
            p.stop()
        self._td.cleanup()

    def test_idempotency_second_call_rejected(self) -> None:
        sw.check_and_reserve_gates(
            event_id=HEX64_A, requester=HEX64_B, channel=UUID_A, reserve_usd=0.1
        )
        with self.assertRaises(RuntimeError) as cm:
            sw.check_and_reserve_gates(
                event_id=HEX64_A, requester=HEX64_B, channel=UUID_A, reserve_usd=0.1
            )
        self.assertIn("idempotency", str(cm.exception).lower())

    def test_lock_reject_does_not_consume_reservation(self) -> None:
        paths = sw._gates_paths()
        lock = sw.ConcurrencyGate(paths["lock"], 1)
        lock.acquire()
        try:
            lock2 = sw.ConcurrencyGate(paths["lock"], 1)
            with self.assertRaises(RuntimeError) as cm:
                lock2.acquire()
            self.assertIn("concurrency", str(cm.exception).lower())
            # Missing gate-state is empty (not an error); nothing reserved.
            self.assertEqual(_gate_idemp(), {})
            self.assertEqual(float(_gate_spend().get("reserved_usd") or 0), 0.0)
        finally:
            lock.release()

    def test_lock_before_reserve_in_run_swarm_gate_path(self) -> None:
        paths = sw._gates_paths()
        held = sw.ConcurrencyGate(paths["lock"], 1)
        held.acquire()
        try:
            with tempfile.TemporaryDirectory() as std:
                with mock.patch.object(sw, "STATE_ROOT", Path(std)):
                    with mock.patch.dict(os.environ, {"OPENAI_API_KEY": FAKE_KEY}):
                        result = sw.run_swarm(
                            "lock order topic",
                            event_id=HEX64_A,
                            requester=HEX64_B,
                            channel=UUID_A,
                            enforce_gates=True,
                        )
                self.assertFalse(result.passed)
                self.assertIn("gate rejected", result.error or "")
                self.assertIn("concurrency", (result.error or "").lower())
                self.assertNotIn(HEX64_A, _gate_idemp())
                self.assertNotIn(HEX64_B, _gate_requesters())
                self.assertEqual(float(_gate_spend().get("reserved_usd") or 0), 0.0)
        finally:
            held.release()


class TestRunDirSuffix(unittest.TestCase):
    def test_unique_suffix_on_collision(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            first = sw._unique_run_dir(root, "20260801T120000Z", "topic")
            first.mkdir()
            second = sw._unique_run_dir(root, "20260801T120000Z", "topic")
            self.assertNotEqual(first, second)
            self.assertTrue(str(second.name).startswith("20260801T120000Z-topic-"))
            self.assertFalse(second.exists())


class TestFutureResultFailures(unittest.TestCase):
    def test_future_exception_becomes_failed_receipt(self) -> None:
        long_content = "finding " * 80

        def fake_retry(**kwargs: Any) -> tuple[str, list[sw.CallReceipt]]:
            role = kwargs.get("role", "")
            if "architecture" in role:
                raise RuntimeError("simulated worker crash")
            rec = sw.CallReceipt(
                role=role,
                model=sw.configured_model(),
                ok=True,
                content_chars=len(long_content),
            )
            rec.cost_usd = 0.001
            rec.total_tokens = 50
            return long_content, [rec]

        with tempfile.TemporaryDirectory() as td:
            state = Path(td) / "state"
            gates = Path(td) / "gates"
            with mock.patch.object(sw, "STATE_ROOT", state):
                with mock.patch.object(sw, "GATES_ROOT", gates):
                    with mock.patch.object(
                        sw, "chat_completions_retry", side_effect=fake_retry
                    ):
                        with mock.patch.dict(os.environ, {"OPENAI_API_KEY": FAKE_KEY}):
                            result = sw.run_swarm(
                                "future crash topic",
                                skip_evidence=True,
                                enforce_gates=False,
                            )
            self.assertFalse(result.passed)
            self.assertIn("fail-closed", result.error or "")
            run = Path(result.run_dir)
            failed = list(run.glob("worker-architecture.FAILED.md"))
            self.assertEqual(len(failed), 1)
            body = failed[0].read_text()
            self.assertIn("future.result exception", body)
            errs = [
                r
                for r in result.receipts
                if r.get("error") and "future.result" in (r.get("error") or "")
            ]
            self.assertTrue(errs)
            # Receipt JSON must not contain the fake key
            receipt_blob = (run / "receipt.json").read_text()
            self.assertNotIn(FAKE_KEY, receipt_blob)


class TestEnforceGatesSharedMode(unittest.TestCase):
    def test_enforce_refuses_skip_and_missing_ids(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            state = Path(td) / "state"
            gates = Path(td) / "gates"
            with mock.patch.object(sw, "STATE_ROOT", state):
                with mock.patch.object(sw, "GATES_ROOT", gates):
                    with mock.patch.dict(os.environ, {"OPENAI_API_KEY": FAKE_KEY}):
                        r1 = sw.run_swarm(
                            "shared skip",
                            skip_evidence=True,
                            event_id=HEX64_A,
                            requester=HEX64_B,
                            channel=UUID_A,
                            enforce_gates=True,
                        )
                        r2 = sw.run_swarm(
                            "shared missing",
                            enforce_gates=True,
                        )
                        r3 = sw.run_swarm(
                            "shared evidence file",
                            evidence_file=Path(td) / "e.md",
                            event_id=HEX64_A,
                            requester=HEX64_B,
                            channel=UUID_A,
                            enforce_gates=True,
                        )
        self.assertFalse(r1.passed)
        self.assertIn("skip-evidence", r1.error or "")
        self.assertFalse(r2.passed)
        self.assertIn("event-id", (r2.error or "").lower())
        self.assertFalse(r3.passed)
        self.assertIn("evidence-file", r3.error or "")


class TestHappyPathMocked(unittest.TestCase):
    def test_ten_workers_plus_synthesis_pass(self) -> None:
        long_content = ("finding about product surface and ops. " * 20).strip()
        brief = (
            "🌐 Last30Days · multi-worker · 2026-08-01\n\n"
            "## What I learned\n"
            " - **Lead.** Detail about the research topic.\n"
            " - **Second.** More detail.\n\n"
            "## KEY PATTERNS\n"
            "1. Pattern one\n"
            "2. Pattern two\n\n"
            "## Buzz use cases\n"
            "1. Use case\n\n"
            "## Risks\n"
            " - Risk one\n"
        ) + ("extra padding for min chars. " * 30)

        def fake_retry(**kwargs: Any) -> tuple[str, list[sw.CallReceipt]]:
            role = kwargs.get("role", "")
            if role.startswith("synthesis"):
                rec = sw.CallReceipt(
                    role=role,
                    model=sw.DEFAULT_MODEL,
                    ok=True,
                    content_chars=len(brief),
                    total_tokens=100,
                    cost_usd=0.01,
                )
                return brief, [rec]
            rec = sw.CallReceipt(
                role=role,
                model=sw.DEFAULT_MODEL,
                ok=True,
                content_chars=len(long_content),
                total_tokens=50,
                cost_usd=0.001,
            )
            return long_content, [rec]

        with tempfile.TemporaryDirectory() as td:
            state = Path(td) / "state"
            gates = Path(td) / "gates"
            with mock.patch.object(sw, "STATE_ROOT", state):
                with mock.patch.object(sw, "GATES_ROOT", gates):
                    with mock.patch.object(
                        sw, "chat_completions_retry", side_effect=fake_retry
                    ):
                        with mock.patch.dict(os.environ, {"OPENAI_API_KEY": FAKE_KEY}):
                            result = sw.run_swarm(
                                "Buzz agent collaboration",
                                skip_evidence=True,
                                enforce_gates=False,
                            )
            self.assertTrue(result.passed, result.error)
            self.assertEqual(result.usable_workers, 10)
            self.assertIn("What I learned", result.brief)
            run = Path(result.run_dir)
            self.assertEqual(stat.S_IMODE(run.stat().st_mode), 0o700)
            workers = list(run.glob("worker-*.md"))
            self.assertEqual(len(workers), 10)
            receipt = (run / "receipt.json").read_text()
            self.assertNotIn(FAKE_KEY, receipt)
            self.assertNotIn("Authorization", receipt)


class TestRedaction(unittest.TestCase):
    def test_redact_key(self) -> None:
        out = sw._redact_secrets(f"Bearer {FAKE_KEY} and key={FAKE_KEY}", FAKE_KEY)
        self.assertNotIn(FAKE_KEY, out)
        self.assertIn("[redacted", out.lower())

    def test_redact_absolute_paths(self) -> None:
        """Receipt/error path scrub: absolute paths must not leak publicly."""
        posix = "FileNotFoundError: [Errno 2] No such file: '/home/alice/.last30days-runs/run-xyz/evidence-brief.md'"
        win = r"cannot open C:\Users\alice\AppData\Local\last30days\run\out.txt"
        file_url = "failed file:///home/alice/secret/key.pem"
        for sample in (posix, win, file_url):
            out = sw._redact_secrets(sample)
            self.assertNotIn("/home/alice", out)
            self.assertNotIn(r"C:\Users\alice", out)
            self.assertNotIn("file:///home/alice", out)
            self.assertIn("[redacted-path]", out)

    def test_safe_error_scrubs_filenotfound_path(self) -> None:
        exc = FileNotFoundError(2, "No such file or directory", "/tmp/secret-run/evidence.md")
        # Python formats as: [Errno 2] No such file or directory: '/tmp/secret-run/evidence.md'
        text = sw._safe_error(exc, FAKE_KEY)
        self.assertNotIn("/tmp/secret-run", text)
        self.assertIn("[redacted-path]", text)

    def test_receipt_error_field_has_no_absolute_path(self) -> None:
        result = sw.SwarmResult(
            topic="t",
            model="m",
            started_at="t0",
            finished_at="t1",
            run_dir="/home/alice/.last30days-runs/run-1",
            evidence_path="/home/alice/.last30days-runs/run-1/evidence-brief.md",
            error=sw._safe_error(
                FileNotFoundError(
                    2,
                    "No such file or directory",
                    "/home/alice/.last30days-runs/run-1/evidence-brief.md",
                )
            ),
            passed=False,
            receipts=[
                {
                    "role": "worker:x",
                    "ok": False,
                    "error": sw._safe_error(
                        OSError("open /var/lib/last30days/x failed")
                    ),
                }
            ],
        )
        payload = sw._receipt_payload(result)
        blob = json.dumps(payload)
        self.assertNotIn("/home/alice", blob)
        self.assertNotIn("/var/lib/last30days", blob)
        # _persist also runs _redact_secrets over the whole receipt blob.
        with tempfile.TemporaryDirectory() as td:
            out = Path(td)
            result.run_dir = str(out)
            sw._persist(out, result)
            receipt = (out / "receipt.json").read_text(encoding="utf-8")
            self.assertNotIn("/home/alice", receipt)
            self.assertNotIn("/var/lib/last30days", receipt)

    def test_no_personal_paths_in_module_source(self) -> None:
        """Production module must not embed host-local fingerprints.

        Allowed absolute homes (if any) are generic placeholders only
        (/home/alice, /home/bob, /home/adopter) — never real usernames.
        """
        src = Path(sw.__file__).read_text(encoding="utf-8")
        allowed_homes = {"/home/alice", "/home/bob", "/home/adopter"}
        for match in re.finditer(r"/home/[A-Za-z0-9_.-]+", src):
            token = match.group(0)
            self.assertIn(
                token,
                allowed_homes,
                f"unexpected home path in module source: {token}",
            )
        # Legacy internal env-file discovery name must not reappear.
        self.assertNotIn("L30D_ENV_FILE", src)


class TestBriefShape(unittest.TestCase):
    def test_looks_like_brief(self) -> None:
        good = "🌐 Last30Days\n## What I learned\n - **x.** y\n## KEY PATTERNS\n1. a\n"
        self.assertTrue(sw._looks_like_brief(good))
        self.assertFalse(sw._looks_like_brief("we need to produce the output format"))


# ---------------------------------------------------------------------------
# HOLD five-fix regressions (argv, topic I/O, transactional gates,
# min-success, minimal receipt). Injection proof is non-negotiable.
# ---------------------------------------------------------------------------


class TestEvidenceArgvTemplate(unittest.TestCase):
    """Fix #1: LAST30DAYS_EVIDENCE_CMD is JSON argv + shell=False only."""

    def test_rejects_shell_string_template(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.parse_evidence_argv_template('echo "{topic}"')
        self.assertIn("JSON array", str(cm.exception))

    def test_rejects_shell_metachar_string(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.parse_evidence_argv_template('my-tool --topic "{topic}"; rm -rf /')
        self.assertIn("JSON array", str(cm.exception))

    def test_rejects_non_array_json(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.parse_evidence_argv_template('{"cmd":"x"}')
        self.assertIn("JSON array", str(cm.exception))

    def test_accepts_json_argv_array(self) -> None:
        tmpl = sw.parse_evidence_argv_template(
            '["my-tool","--topic","{topic}","--days","{days}"]'
        )
        self.assertEqual(tmpl, ["my-tool", "--topic", "{topic}", "--days", "{days}"])

    def test_topic_is_single_opaque_argv_element(self) -> None:
        evil = 'x"; echo PWNED > /tmp/should-not-exist; echo "y'
        argv = sw.render_evidence_argv(
            ["printer", "--topic", "{topic}", "--days", "{days}"],
            topic=evil,
            days=30,
            out_dir="/tmp/out",
        )
        self.assertEqual(argv[0], "printer")
        self.assertEqual(argv[1], "--topic")
        self.assertEqual(argv[2], evil)
        self.assertEqual(len(argv), 5)

    def test_malicious_topic_does_not_execute(self) -> None:
        """Non-negotiable injection regression (Fable P0 reproduction shape).

        Under the old shell=True + .format() path, a topic containing
        shell metacharacters would execute. Now the topic is one opaque
        argv element under shell=False — the proof file must NOT appear.
        """
        with tempfile.TemporaryDirectory() as td:
            proof = Path(td) / "pwned.proof"
            out_dir = Path(td) / "run"
            out_dir.mkdir()
            # Topic shaped like the confirmed P0: closes a quote and runs a command.
            evil_topic = f'x"; echo INJECTED > {proof}; echo "y'
            # JSON argv: child prints topic from one opaque argv element to stdout.
            # python -c is allowed; shell-interpreter -c with placeholders is not.
            printer = "import sys; print('brief for:', sys.argv[1])"
            cmd = json.dumps([sys.executable, "-c", printer, "{topic}"])
            with mock.patch.dict(
                os.environ,
                {
                    "LAST30DAYS_EVIDENCE_CMD": cmd,
                    "LAST30DAYS_EVIDENCE_TIMEOUT": "30",
                    "OPENAI_API_KEY": FAKE_KEY,
                },
                clear=False,
            ):
                path = sw.gather_evidence(
                    evil_topic, days=30, out_dir=out_dir, key=FAKE_KEY
                )
            self.assertFalse(
                proof.exists(),
                "injection executed — shell=True regression reintroduced",
            )
            text = path.read_text(encoding="utf-8")
            # Topic survived as opaque data inside the brief, not as shell.
            self.assertIn("brief for:", text)
            self.assertIn(evil_topic, text)

    def test_rejects_shell_interpreter_c_with_topic_placeholder(self) -> None:
        """A5: ["sh","-c","{topic}"] must be rejected (operator footgun)."""
        for tmpl in (
            '["sh","-c","{topic}"]',
            '["/bin/bash","-c","echo {topic}"]',
            '["zsh","-c","{topic}; id"]',
            '["dash","-c","printf %s {topic}"]',
            '["cmd","/c","echo {topic}"]',
            '["powershell","-Command","Write-Output {topic}"]',
            '["pwsh","-command","{topic}"]',
        ):
            with self.assertRaises(RuntimeError, msg=tmpl) as cm:
                sw.parse_evidence_argv_template(tmpl)
            msg = str(cm.exception).lower()
            self.assertTrue(
                "shell-interpreter" in msg or "-c" in msg or "placeholder" in msg,
                msg,
            )

    def test_a5_sh_c_topic_does_not_execute_via_gather(self) -> None:
        """End-to-end: A5 template is rejected before any subprocess runs."""
        with tempfile.TemporaryDirectory() as td:
            proof = Path(td) / "a5-pwned.proof"
            out_dir = Path(td) / "run"
            out_dir.mkdir()
            evil = f"echo A5_INJECTED > {proof}"
            cmd = json.dumps(["sh", "-c", "{topic}"])
            with mock.patch.dict(
                os.environ,
                {
                    "LAST30DAYS_EVIDENCE_CMD": cmd,
                    "LAST30DAYS_EVIDENCE_TIMEOUT": "30",
                    "OPENAI_API_KEY": FAKE_KEY,
                },
                clear=False,
            ):
                with self.assertRaises(RuntimeError) as cm:
                    sw.gather_evidence(evil, days=30, out_dir=out_dir, key=FAKE_KEY)
            self.assertFalse(proof.exists(), "A5 shell -c template executed")
            self.assertIn("shell-interpreter", str(cm.exception).lower())

    def test_python_c_with_placeholder_still_allowed(self) -> None:
        """Non-shell interpreters may use -c; only shell argv[0] is blocked."""
        printer = "import sys; print(sys.argv[1])"
        tmpl = sw.parse_evidence_argv_template(
            json.dumps([sys.executable, "-c", printer, "{topic}"])
        )
        self.assertEqual(tmpl[0], sys.executable)
        self.assertIn("{topic}", tmpl)

    def test_gather_evidence_rejects_shell_template_env(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out_dir = Path(td)
            with mock.patch.dict(
                os.environ,
                {
                    "LAST30DAYS_EVIDENCE_CMD": 'echo "{topic}"',
                    "OPENAI_API_KEY": FAKE_KEY,
                },
                clear=False,
            ):
                with self.assertRaises(RuntimeError) as cm:
                    sw.gather_evidence(
                        "safe topic", days=30, out_dir=out_dir, key=FAKE_KEY
                    )
            self.assertIn("JSON array", str(cm.exception))

    def test_subprocess_called_with_shell_false(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out_dir = Path(td)
            evil = "topic; rm -rf /"
            cmd = json.dumps([sys.executable, "-c", "print('ok')", "{topic}"])
            with mock.patch.dict(
                os.environ,
                {
                    "LAST30DAYS_EVIDENCE_CMD": cmd,
                    "LAST30DAYS_EVIDENCE_TIMEOUT": "30",
                    "OPENAI_API_KEY": FAKE_KEY,
                },
                clear=False,
            ):
                with mock.patch.object(sw.subprocess, "run") as run:
                    run.return_value = mock.Mock(
                        stdout="evidence body here\n",
                        stderr="",
                        returncode=0,
                    )
                    sw.gather_evidence(
                        evil, days=7, out_dir=out_dir, key=FAKE_KEY
                    )
                    self.assertTrue(run.called)
                    kwargs = run.call_args.kwargs
                    self.assertIs(kwargs.get("shell"), False)
                    argv = run.call_args.args[0]
                    self.assertIsInstance(argv, list)
                    self.assertIn(evil, argv)


class TestTopicOpaqueInput(unittest.TestCase):
    """Fix #2: topic via --topic-file / --topic-stdin (no shell interpolation)."""

    def test_topic_file(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "topic.txt"
            p.write_text('hello "$(rm -rf /)" world\n', encoding="utf-8")
            got = sw.resolve_topic_input(
                positional=[], topic_file=p, topic_stdin=False
            )
            self.assertEqual(got, 'hello "$(rm -rf /)" world\n')

    def test_topic_stdin(self) -> None:
        evil = "topic with `id` and $(whoami) and ; reboot"
        with mock.patch.object(sw.sys, "stdin", mock.Mock(read=lambda: evil)):
            got = sw.resolve_topic_input(
                positional=[], topic_file=None, topic_stdin=True
            )
        self.assertEqual(got, evil)

    def test_positional_still_works(self) -> None:
        got = sw.resolve_topic_input(
            positional=["Buzz", "agents"], topic_file=None, topic_stdin=False
        )
        self.assertEqual(got, "Buzz agents")

    def test_rejects_multiple_sources(self) -> None:
        with self.assertRaises(RuntimeError) as cm:
            sw.resolve_topic_input(
                positional=["x"], topic_file=Path("/tmp/t"), topic_stdin=False
            )
        self.assertIn("exactly one", str(cm.exception))

    def test_docs_forbid_shell_quoted_topic_pattern(self) -> None:
        """Persona/skill/README must not document python3 … \"<topic>\"."""
        root = Path(sw.__file__).resolve().parent.parent
        dangerous = re.compile(
            r'python3\s+[^\n]*last30days\.py[^\n]*["\']\{?topic\}?["\']'
            r'|python3\s+[^\n]*["\']\$?\{?TOPIC\}?["\']',
            re.IGNORECASE,
        )
        for rel in (
            "agents/last30days.persona.md",
            "skills/last30days/SKILL.md",
            "README.md",
            "instructions.md",
        ):
            text = (root / rel).read_text(encoding="utf-8")
            self.assertIsNone(
                dangerous.search(text),
                f"{rel} still documents shell-quoted topic argv",
            )
            self.assertRegex(
                text,
                r"--topic-stdin|--topic-file",
                f"{rel} must document opaque topic input",
            )


class TestTransactionalGates(unittest.TestCase):
    """Fix #3: validate all gates first; rejection consumes nothing.

    Final-round: one consolidated gate-state.json, atomic temp+fsync+replace,
    unparseable state fail-CLOSED.
    """

    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.gates = Path(self._td.name)
        self._patchers = [
            mock.patch.object(sw, "GATES_ROOT", self.gates),
            mock.patch.object(sw, "GLOBAL_DAILY_SPEND_USD", 1.0),
            mock.patch.object(sw, "RESERVE_USD", 0.5),
            mock.patch.object(sw, "COOLDOWN_S", 300),
            mock.patch.object(sw, "DAILY_QUOTA", 2),
        ]
        for p in self._patchers:
            p.start()

    def tearDown(self) -> None:
        for p in self._patchers:
            p.stop()
        self._td.cleanup()

    def test_spend_deny_consumes_no_idempotency_or_quota(self) -> None:
        _seed_gate_state(spend={"total_usd": 0.9, "reserved_usd": 0.0})
        with self.assertRaises(RuntimeError) as cm:
            sw.check_and_reserve_gates(
                event_id=HEX64_A,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )
        self.assertIn("reservation denied", str(cm.exception))
        self.assertNotIn(HEX64_A, _gate_idemp())
        self.assertNotIn(HEX64_B, _gate_requesters())
        spend = _gate_spend()
        self.assertEqual(float(spend.get("reserved_usd") or 0), 0.0)
        self.assertAlmostEqual(float(spend.get("total_usd") or 0), 0.9)

    def test_cooldown_deny_consumes_no_idempotency_or_spend(self) -> None:
        now = time.time()
        _seed_gate_state(
            requesters={HEX64_B: {"count": 0, "last_ts": now, "runs": []}},
            spend={"total_usd": 0.0, "reserved_usd": 0.0},
        )
        with self.assertRaises(RuntimeError) as cm:
            sw.check_and_reserve_gates(
                event_id=HEX64_A,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )
        self.assertIn("cooldown", str(cm.exception).lower())
        self.assertNotIn(HEX64_A, _gate_idemp())
        self.assertEqual(float(_gate_spend().get("reserved_usd") or 0), 0.0)
        # Cooldown path must not bump count.
        self.assertEqual(int(_gate_requesters()[HEX64_B]["count"]), 0)

    def test_quota_deny_consumes_no_idempotency_or_spend(self) -> None:
        _seed_gate_state(
            requesters={HEX64_B: {"count": 2, "last_ts": 0.0, "runs": []}},
            spend={"total_usd": 0.0, "reserved_usd": 0.0},
        )
        with self.assertRaises(RuntimeError) as cm:
            sw.check_and_reserve_gates(
                event_id=HEX64_C,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )
        self.assertIn("quota", str(cm.exception).lower())
        self.assertNotIn(HEX64_C, _gate_idemp())
        self.assertEqual(float(_gate_spend().get("reserved_usd") or 0), 0.0)

    def test_success_persists_all_after_validation(self) -> None:
        _seed_gate_state(spend={"total_usd": 0.1, "reserved_usd": 0.0})
        meta = sw.check_and_reserve_gates(
            event_id=HEX64_A,
            requester=HEX64_B,
            channel=UUID_A,
            reserve_usd=0.5,
        )
        self.assertEqual(meta["spend_reserved_this_run"], 0.5)
        self.assertIn(HEX64_A, _gate_idemp())
        self.assertEqual(int(_gate_requesters()[HEX64_B]["count"]), 1)
        self.assertAlmostEqual(float(_gate_spend()["reserved_usd"]), 0.5)
        # Single consolidated file — not three independent JSON files.
        paths = sw._gates_paths()
        self.assertTrue(paths["state"].is_file())
        self.assertEqual(paths["state"].name, sw.GATE_STATE_FILENAME)
        self.assertFalse((self.gates / "idempotency.json").exists())
        self.assertFalse((self.gates / "requesters.json").exists())
        self.assertFalse((self.gates / "spend.json").exists())

    def test_torn_json_fail_closed(self) -> None:
        """Corrupt / partial JSON must NOT be treated as empty {}."""
        paths = sw._gates_paths()
        paths["state"].write_text("{not valid json partial", encoding="utf-8")
        with self.assertRaises(RuntimeError) as cm:
            sw._load_gate_state(paths["state"])
        self.assertIn("fail-closed", str(cm.exception).lower())
        with self.assertRaises(RuntimeError) as cm2:
            sw.check_and_reserve_gates(
                event_id=HEX64_A,
                requester=HEX64_B,
                channel=UUID_A,
                reserve_usd=0.5,
            )
        self.assertIn("fail-closed", str(cm2.exception).lower())

    def test_atomic_save_replace_failure_preserves_prior_state(self) -> None:
        """If os.replace fails mid-write, prior gate-state bytes stay intact."""
        _seed_gate_state(spend={"total_usd": 0.25, "reserved_usd": 0.0})
        paths = sw._gates_paths()
        prior = paths["state"].read_text(encoding="utf-8")
        self.assertIn("0.25", prior)

        def boom(*_a: Any, **_k: Any) -> None:
            raise OSError("simulated replace failure")

        with mock.patch("os.replace", side_effect=boom):
            with self.assertRaises(OSError):
                sw.check_and_reserve_gates(
                    event_id=HEX64_A,
                    requester=HEX64_B,
                    channel=UUID_A,
                    reserve_usd=0.5,
                )
        after = paths["state"].read_text(encoding="utf-8")
        self.assertEqual(after, prior)
        # No partial reservation consumed.
        self.assertNotIn(HEX64_A, _gate_idemp())
        self.assertAlmostEqual(float(_gate_spend()["total_usd"]), 0.25)
        self.assertEqual(float(_gate_spend().get("reserved_usd") or 0), 0.0)


class TestMinSuccessSharedMode(unittest.TestCase):
    """Fix #4: under --enforce-gates, min-success == worker count always."""

    def test_enforce_gates_ignores_lower_min_success_knob(self) -> None:
        with mock.patch.object(sw, "MIN_SUCCESS", 3):
            self.assertEqual(sw.resolve_min_success(10, enforce_gates=True), 10)
            self.assertEqual(sw.resolve_min_success(7, enforce_gates=True), 7)

    def test_owner_mode_may_use_knob(self) -> None:
        with mock.patch.object(sw, "MIN_SUCCESS", 3):
            self.assertEqual(sw.resolve_min_success(10, enforce_gates=False), 3)

    def test_run_swarm_shared_min_equals_workers(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            state = Path(td) / "state"
            gates = Path(td) / "gates"
            with mock.patch.object(sw, "STATE_ROOT", state):
                with mock.patch.object(sw, "GATES_ROOT", gates):
                    with mock.patch.object(sw, "MIN_SUCCESS", 1):
                        with mock.patch.object(sw, "GLOBAL_DAILY_SPEND_USD", 0.0):
                            with mock.patch.dict(
                                os.environ, {"OPENAI_API_KEY": FAKE_KEY}
                            ):
                                # Spend ceiling 0 forces early gate reject after
                                # min_success is already computed.
                                result = sw.run_swarm(
                                    "min success check",
                                    event_id=HEX64_A,
                                    requester=HEX64_B,
                                    channel=UUID_A,
                                    enforce_gates=True,
                                )
        self.assertEqual(result.min_success, result.worker_total)
        self.assertEqual(result.worker_total, 10)


class TestMinimalReceiptSchema(unittest.TestCase):
    """Fix #5: receipt.json is metadata only — no topic/brief/paths/gates."""

    def test_receipt_payload_excludes_sensitive_fields(self) -> None:
        result = sw.SwarmResult(
            topic="secret research topic about acme",
            model="deepseek/deepseek-v4-pro",
            started_at="2026-08-01T00:00:00+00:00",
            finished_at="2026-08-01T00:01:00+00:00",
            run_dir="/home/alice/.last30days-runs/run-xyz",
            evidence_path="/home/alice/.last30days-runs/run-xyz/evidence-brief.md",
            worker_total=10,
            usable_workers=10,
            min_success=10,
            total_tokens=1234,
            total_cost_usd=0.042,
            brief="FULL BRIEF TEXT THAT MUST NOT LEAK INTO RECEIPT",
            passed=True,
            gates={
                "event_id": HEX64_A,
                "requester": HEX64_B,
                "channel": UUID_A,
            },
            receipts=[
                {
                    "role": "worker:product_surface",
                    "ok": True,
                    "model": "deepseek/deepseek-v4-pro",
                    "provider": "OpenRouter",
                    "prompt_tokens": 10,
                    "completion_tokens": 20,
                    "total_tokens": 30,
                    "cost_usd": 0.001,
                    "latency_s": 1.2,
                    "attempt": 1,
                    "finish_reason": "stop",
                    "error": None,
                }
            ],
        )
        payload = sw._receipt_payload(result)
        blob = json.dumps(payload)
        self.assertNotIn("secret research topic", blob)
        self.assertNotIn("FULL BRIEF", blob)
        self.assertNotIn("/home/alice", blob)
        self.assertNotIn(HEX64_A, blob)
        self.assertNotIn(HEX64_B, blob)
        self.assertNotIn(UUID_A, blob)
        self.assertNotIn("run_dir", payload)
        self.assertNotIn("topic", payload)
        self.assertNotIn("brief", payload)
        self.assertNotIn("gates", payload)
        self.assertNotIn("evidence_path", payload)
        # Required metadata present
        self.assertEqual(payload["model"], "deepseek/deepseek-v4-pro")
        self.assertEqual(payload["status"], "ok")
        self.assertEqual(payload["total_tokens"], 1234)
        self.assertEqual(payload["total_cost_usd"], 0.042)
        self.assertIn("calls", payload)

    def test_persist_splits_receipt_and_context(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            out = Path(td)
            result = sw.SwarmResult(
                topic="private topic string",
                model="m",
                started_at="t0",
                finished_at="t1",
                run_dir=str(out),
                evidence_path=str(out / "evidence-brief.md"),
                worker_total=10,
                usable_workers=9,
                min_success=10,
                brief="private brief body",
                passed=False,
                error="fail-closed",
                gates={"event_id": HEX64_A, "requester": HEX64_B, "channel": UUID_A},
                receipts=[],
            )
            sw._persist(out, result)
            receipt = (out / "receipt.json").read_text(encoding="utf-8")
            ctx = (out / "run-context.json").read_text(encoding="utf-8")
            self.assertNotIn("private topic string", receipt)
            self.assertNotIn("private brief body", receipt)
            self.assertNotIn(HEX64_A, receipt)
            self.assertIn("private topic string", ctx)
            self.assertIn(HEX64_A, ctx)
            self.assertIn("fail-closed", receipt)


def main() -> int:
    loader = unittest.TestLoader()
    suite = loader.loadTestsFromModule(sys.modules[__name__])
    runner = unittest.TextTestRunner(verbosity=2)
    result = runner.run(suite)
    print(
        f"\nSUMMARY: ran={result.testsRun} "
        f"failures={len(result.failures)} errors={len(result.errors)} "
        f"skipped={len(result.skipped)}"
    )
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
