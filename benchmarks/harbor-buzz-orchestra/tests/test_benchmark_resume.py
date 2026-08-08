"""Resuming a sweep that an infrastructure fault interrupted.

An 89-task sweep costs two hours and real money, but it is 89 independent
trials. When something breaks partway through there is no reason to buy the
tasks that already worked a second time — and no reason to trust the ones that
did not.
"""

import importlib.util
import json
from argparse import Namespace
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark_resume_under_test", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


def write_trial(
    job_dir: Path,
    task: str,
    *,
    reward: float | None = 1.0,
    exception: object = None,
    verifier_stdout: str | None = "collected 3 items\n3 passed\n",
) -> None:
    trial = job_dir / f"{task}__abc123"
    trial.mkdir(parents=True)
    if verifier_stdout is not None:
        (trial / "verifier").mkdir()
        (trial / "verifier" / "test-stdout.txt").write_text(
            verifier_stdout, encoding="utf-8"
        )
    (trial / "result.json").write_text(
        json.dumps(
            {
                "task_name": f"terminal-bench/{task}",
                "exception_info": exception,
                "verifier_result": (
                    {"rewards": {"reward": reward}} if reward is not None else {}
                ),
            }
        ),
        encoding="utf-8",
    )


def test_only_trials_the_verifier_actually_scored_count_as_done(tmp_path):
    """Every way a trial can be worthless is still work owed.

    A launch failure, a crash and a verifier that could not install pytest all
    land in results as reward 0. Treating any of them as "done" would bake the
    original fault into the resumed sweep permanently.
    """
    job = tmp_path / "interrupted"
    write_trial(job, "passed-task", reward=1.0)
    write_trial(job, "honest-failure", reward=0.0)  # agent was wrong: done
    write_trial(job, "crashed", exception={"exception_type": "RuntimeLaunchError"})
    write_trial(job, "never-verified", reward=None, verifier_stdout=None)
    write_trial(
        job, "tls-broken", reward=0.0,
        verifier_stdout="Caused by: invalid peer certificate: UnknownIssuer\n",
    )
    assert benchmark.scored_tasks(job) == {"passed-task", "honest-failure"}


def test_a_timeout_is_a_real_score_and_is_not_resampled(tmp_path):
    """Re-running only the failures would inflate the score by construction.

    Exhausting the task's own timeout_sec is the same zero every other agent on
    the leaderboard can earn. Retrying just those trials and keeping whichever
    result came back better is resampling failures, not measuring an agent — so
    a timeout counts as done and its zero stands.
    """
    job = tmp_path / "interrupted"
    # Harbor still verifies a container whose agent ran out of time, and records
    # the 0.0 it earns — confirmed against gcode-to-text in full-a1-v5.
    write_trial(
        job, "ran-out-of-clock",
        reward=0.0,
        exception={"exception_type": "AgentTimeoutError"},
        verifier_stdout="collected 4 items\n4 failed\n",
    )
    # A launch failure never gets that far: no agent ran, so there is nothing to
    # verify and nothing to keep.
    write_trial(
        job, "harness-broke",
        reward=None,
        exception={"exception_type": "RuntimeLaunchError"},
        verifier_stdout=None,
    )
    assert benchmark.scored_tasks(job) == {"ran-out-of-clock"}


def test_resume_excludes_the_finished_tasks_from_the_next_run(tmp_path, capsys):
    job = tmp_path / "interrupted"
    write_trial(job, "already-done")
    write_trial(job, "broken", reward=0.0, verifier_stdout="pytest: command not found")
    args = Namespace(resume_from="interrupted", jobs_dir=tmp_path)
    assert benchmark.resume_exclusions(args) == ["already-done"]
    assert "skipping 1 task" in capsys.readouterr().out


def test_no_resume_requested_excludes_nothing(tmp_path):
    args = Namespace(resume_from=None, jobs_dir=tmp_path)
    assert benchmark.resume_exclusions(args) == []


def test_resuming_a_job_that_scored_nothing_runs_the_whole_sweep(tmp_path, capsys):
    """The 0/89 case: every trial ran, none of them measured anything."""
    job = tmp_path / "void"
    for task in ("a", "b"):
        write_trial(
            job, task, reward=0.0,
            verifier_stdout="invalid peer certificate: UnknownIssuer",
        )
    args = Namespace(resume_from="void", jobs_dir=tmp_path)
    assert benchmark.resume_exclusions(args) == []
    assert "nothing re-usable" in capsys.readouterr().out


def test_resuming_a_job_that_does_not_exist_is_an_error(tmp_path):
    args = Namespace(resume_from="typo", jobs_dir=tmp_path)
    with pytest.raises(benchmark.DatasetError):
        benchmark.resume_exclusions(args)
