"""A relay 429 must not end a trial — see ``buzz_cli.MAX_ATTEMPTS``."""

from __future__ import annotations

import subprocess

import pytest
from harbor_buzz_testbed import buzz_cli
from harbor_buzz_testbed.buzz_cli import MAX_ATTEMPTS, BuzzCli, BuzzCliError

RATE_LIMITED = (
    '{"error":"relay_error","message":"relay error 429: rate-limited: '
    'quota exceeded; retry in 0s","retryable":true}'
)


def completed(returncode: int, stdout: str = "", stderr: str = ""):
    return subprocess.CompletedProcess(
        args=["buzz"], returncode=returncode, stdout=stdout, stderr=stderr
    )


@pytest.fixture
def cli(monkeypatch):
    """A CLI whose sleeps are free, so backoff does not slow the suite."""
    monkeypatch.setattr(buzz_cli.time, "sleep", lambda _seconds: None)
    return BuzzCli("http://relay", "sec", "tag")


def responses(monkeypatch, cli, queue):
    """Feed ``cli`` a scripted sequence of process results; record the calls."""
    seen: list[tuple[str, ...]] = []

    def fake(*args: str):
        seen.append(args)
        return queue.pop(0)

    monkeypatch.setattr(cli, "_invoke", fake)
    return seen


def test_a_rate_limited_call_is_retried_rather_than_failing_the_trial(
    monkeypatch, cli
):
    """The relay says ``retryable`` and means it: the same call then succeeds.

    This is the whole point of the retry. A 429 during provisioning used to
    propagate as a trial failure, which scores a Terminal-Bench task zero for a
    reason that has nothing to do with the agent being measured.
    """
    seen = responses(
        monkeypatch,
        cli,
        [
            completed(2, stderr=RATE_LIMITED),
            completed(2, stderr=RATE_LIMITED),
            completed(0, stdout='{"channel_id": "abc"}'),
        ],
    )

    assert cli.create_private_channel("task", "run=1") == "abc"
    assert len(seen) == 3


def test_a_permanent_error_is_not_retried(monkeypatch, cli):
    """Exit 2 covers permanent relay errors too, and those must stay fatal.

    Retrying a misconfiguration would convert an instant, legible failure into
    a slow one, and — since provisioning runs inside the trial's own clock —
    could spend the task's budget before the agent ever starts.
    """
    seen = responses(
        monkeypatch, cli, [completed(2, stderr='{"error":"channel_not_found"}')]
    )

    with pytest.raises(BuzzCliError, match="channel_not_found"):
        cli.run("channels", "archive", "--channel", "gone")
    assert len(seen) == 1


def test_retries_are_bounded_and_the_last_failure_is_reported(monkeypatch, cli):
    """A relay that is down for good must not retry forever."""
    seen = responses(
        monkeypatch, cli, [completed(2, stderr=RATE_LIMITED)] * MAX_ATTEMPTS
    )

    with pytest.raises(BuzzCliError, match=f"after {MAX_ATTEMPTS} attempts"):
        cli.run("channels", "create")
    assert len(seen) == MAX_ATTEMPTS


def test_concurrent_callers_do_not_retry_in_lockstep():
    """Jitter is what stops a retry from rebuilding the burst that caused it.

    Every trial rejected by one counter is rejected in the same instant, so a
    deterministic delay would send them all back together.
    """
    delays = {buzz_cli._retry_delay(RATE_LIMITED, 1) for _ in range(50)}

    assert len(delays) > 1


def test_backoff_outlasts_the_relays_rate_limit_window():
    """``retry in 0s`` is honest but useless — the floor has to come from us.

    The relay's window is 60s; the delays have to add up to more than that or
    every attempt lands inside the window that is already rejecting them.
    """
    total = sum(
        buzz_cli._retry_delay(RATE_LIMITED, attempt)
        for attempt in range(1, MAX_ATTEMPTS)
    )

    assert total > 60


def test_an_explicit_retry_hint_is_respected_when_it_exceeds_backoff():
    """A relay asking for 30s gets at least 30s, not the 1s backoff would give."""
    hinted = "rate-limited: quota exceeded; retry in 30s"

    assert buzz_cli._retry_delay(hinted, 1) >= 30
