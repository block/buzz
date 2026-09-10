#!/usr/bin/env python3
"""Regression tests for plugin-browser-smoke.py helpers.

Standalone (no pytest dependency): `python3 desktop/scripts/plugin-browser-smoke_test.py`.
Exercises the production module directly by importing the sibling module via
its file path (its filename has a hyphen, so it cannot be `import`ed
normally).
"""

import importlib.util
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from unittest.mock import patch

MODULE_PATH = Path(__file__).resolve().parent / "plugin-browser-smoke.py"


def load_module():
    spec = importlib.util.spec_from_file_location("plugin_browser_smoke", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check(label: str, condition: bool) -> bool:
    print(f"{'PASS' if condition else 'FAIL'}: {label}")
    return condition


def outcomes(permissions, kind):
    return [report["outcome"] for report in permissions.get(kind, [])]


class FakeStdout:
    def __init__(self, fd: int = 7):
        self._fd = fd
        self.closed = False

    def fileno(self) -> int:
        return self._fd

    def close(self) -> None:
        self.closed = True


class FakeProc:
    """Stands in for `subprocess.Popen`'s return value under full control."""

    def __init__(self, wait_results):
        self.stdout = FakeStdout()
        self.wait_calls: list = []
        self._wait_results = list(wait_results)
        self.kill_called = 0

    def poll(self):
        return None

    def kill(self) -> None:
        self.kill_called += 1

    def wait(self, timeout=None):
        self.wait_calls.append(timeout)
        result = self._wait_results.pop(0)
        if isinstance(result, BaseException):
            raise result
        return result


def run_with_fakes(cmd, timeout, max_bytes, module, fake_proc, monotonic_values, select_results, read_results):
    with patch.object(module.subprocess, "Popen", return_value=fake_proc), patch.object(
        module.time, "monotonic", side_effect=monotonic_values
    ), patch.object(module.select, "select", side_effect=select_results), patch.object(
        module.os, "read", side_effect=read_results
    ):
        return module.run_bounded_stdout(cmd, timeout=timeout, max_bytes=max_bytes)


def main() -> int:
    module = load_module()
    ok = True

    # Evidence is never overwritten: an earlier failure and a later
    # "rejected" report from a subsequent /home load must both be visible.
    state = module.FixtureState()
    state.record_permission("audio", "granted", 1)
    state.record_permission("audio", "rejected", 2)
    _, _, permissions = state.snapshot()
    ok &= check(
        "earlier failure is not erased by a later 'rejected' report",
        outcomes(permissions, "audio") == ["granted", "rejected"],
    )

    # A first "rejected" report followed by a later real regression must be
    # captured, not hidden because a "good" outcome was already recorded.
    state = module.FixtureState()
    state.record_permission("video", "rejected", 1)
    state.record_permission("video", "timeout", 2)
    _, _, permissions = state.snapshot()
    ok &= check(
        "a later regression after an earlier 'rejected' is still recorded",
        outcomes(permissions, "video") == ["rejected", "timeout"],
    )

    # The ordinary all-rejected path still works, with an exact count.
    state = module.FixtureState()
    state.record_permission("audio", "rejected", 1)
    state.record_permission("audio", "rejected", 2)
    state.record_permission("video", "rejected", 1)
    state.record_permission("video", "rejected", 2)
    _, _, permissions = state.snapshot()
    ok &= check(
        "all-rejected two-load run reports rejected for every kind",
        outcomes(permissions, "audio") == ["rejected", "rejected"]
        and outcomes(permissions, "video") == ["rejected", "rejected"],
    )

    # next_home_load() hands out a strictly increasing, 1-based counter.
    state = module.FixtureState()
    ok &= check("first home load is 1", state.next_home_load() == 1)
    ok &= check("second home load is 2", state.next_home_load() == 2)

    # --- run_bounded_stdout: fully deterministic, mocked-clock/IO cases ---
    # These fake subprocess.Popen/time.monotonic/select.select/os.read so
    # each branch is forced deterministically, with no dependency on real
    # process startup time, real sleeps, or wall-clock races.
    cmd = ["fake", "cmd"]

    # Normal completion: one data chunk, then EOF, then a clean wait().
    fake_proc = FakeProc(wait_results=[0])
    out = run_with_fakes(
        cmd,
        timeout=5,
        max_bytes=1024,
        module=module,
        fake_proc=fake_proc,
        monotonic_values=[100, 101, 102, 103],
        select_results=[([fake_proc.stdout], [], []), ([fake_proc.stdout], [], [])],
        read_results=[b"hello\n", b""],
    )
    ok &= check(
        "run_bounded_stdout: normal completion decodes output and reaps cleanly",
        out == "hello\n"
        and fake_proc.wait_calls == [2]
        and fake_proc.kill_called == 0
        and fake_proc.stdout.closed,
    )

    # Post-EOF, still-alive child: EOF observed, but proc.wait() on the
    # remaining budget times out — must kill, reap, and propagate the
    # *outer* function timeout, not the inner remaining-budget value.
    fake_proc = FakeProc(wait_results=[subprocess.TimeoutExpired(cmd, 2), 0])
    raised = None
    try:
        run_with_fakes(
            cmd,
            timeout=5,
            max_bytes=1024,
            module=module,
            fake_proc=fake_proc,
            monotonic_values=[100, 101, 103],
            select_results=[([fake_proc.stdout], [], [])],
            read_results=[b""],
        )
    except subprocess.TimeoutExpired as exc:
        raised = exc
    ok &= check(
        "run_bounded_stdout: EOF'd-but-alive child is killed, reaped, and TimeoutExpired(timeout) propagates",
        raised is not None
        and raised.timeout == 5
        and fake_proc.wait_calls == [2, 5]
        and fake_proc.kill_called == 1
        and fake_proc.stdout.closed,
    )

    # Byte-cap overflow: must kill+reap and raise RuntimeError without ever
    # attempting a normal-completion wait().
    fake_proc = FakeProc(wait_results=[0])
    raised = None
    try:
        run_with_fakes(
            cmd,
            timeout=5,
            max_bytes=10,
            module=module,
            fake_proc=fake_proc,
            monotonic_values=[100, 101],
            select_results=[([fake_proc.stdout], [], [])],
            read_results=[b"x" * 20],
        )
    except RuntimeError as exc:
        raised = exc
    ok &= check(
        "run_bounded_stdout: byte-cap overflow is killed, reaped, and raises RuntimeError",
        raised is not None
        and fake_proc.wait_calls == [5]
        and fake_proc.kill_called == 1
        and fake_proc.stdout.closed,
    )

    # Unexpected read error: must still kill+reap the child rather than
    # leaking it, and must propagate the original exception unchanged.
    fake_proc = FakeProc(wait_results=[0])
    raised = None
    try:
        run_with_fakes(
            cmd,
            timeout=5,
            max_bytes=1024,
            module=module,
            fake_proc=fake_proc,
            monotonic_values=[100, 101],
            select_results=[([fake_proc.stdout], [], [])],
            read_results=[OSError("boom")],
        )
    except OSError as exc:
        raised = exc
    ok &= check(
        "run_bounded_stdout: an unexpected read error still kills, reaps, and propagates",
        raised is not None
        and str(raised) == "boom"
        and fake_proc.wait_calls == [5]
        and fake_proc.kill_called == 1
        and fake_proc.stdout.closed,
    )

    # --- Supplementary real-subprocess smoke tests ---
    # These exercise run_bounded_stdout against real child processes for
    # basic sanity (real Popen/select/os.read wiring), but are not relied on
    # to prove any single branch — the mocked cases above do that
    # deterministically. A generous timeout keeps these non-flaky.
    normal_out = module.run_bounded_stdout(
        [sys.executable, "-c", "print('hello')"], timeout=5, max_bytes=1024
    )
    ok &= check(
        "smoke: run_bounded_stdout returns normal small output from a real child",
        normal_out.strip() == "hello",
    )

    oversized_raised = False
    try:
        module.run_bounded_stdout(
            [sys.executable, "-c", "print('x' * 100)"], timeout=5, max_bytes=10
        )
    except RuntimeError:
        oversized_raised = True
    ok &= check(
        "smoke: a real child exceeding the byte cap is killed and raises",
        oversized_raised,
    )

    # (The post-EOF-but-alive child branch is covered deterministically above
    # by the mocked case — a real-subprocess equivalent would depend on child
    # startup timing and PID reuse after reap, so it isn't duplicated here.)

    # assert_registry_and_processes_clean must run cleanly end to end against
    # the real `ps` on this machine (no mocking) when there's nothing to find.
    with tempfile.TemporaryDirectory() as tmp:
        clean_raised = False
        try:
            module.assert_registry_and_processes_clean(Path(tmp))
        except SystemExit:
            clean_raised = True
        ok &= check(
            "assert_registry_and_processes_clean passes against the real process table",
            not clean_raised,
        )

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
