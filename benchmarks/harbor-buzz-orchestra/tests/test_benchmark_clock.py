"""Giving the agent more clock than Terminal-Bench does, on purpose.

The study asks what a team can solve; a deadline that stops a still-working
agent answers a different question. Harbor scales every phase timeout by a
multiplier, which makes the resulting job unsubmittable and the score local —
both fine, as long as nothing quietly re-imposes the old limit and nothing
reports the number as though it were a leaderboard result.
"""

import importlib.util
from argparse import Namespace
from pathlib import Path

import pytest
import yaml

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "benchmark.py"
_spec = importlib.util.spec_from_file_location("benchmark_clock_under_test", _SCRIPT)
benchmark = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(benchmark)


def manifest_with_budget(tmp_path: Path, seconds: int) -> Path:
    path = tmp_path / "condition.yaml"
    path.write_text(
        yaml.safe_dump({"trial_budget": {"timeout_seconds": seconds}}),
        encoding="utf-8",
    )
    return path


def test_the_shipped_default_is_the_generous_clock():
    """Chosen so the study's headline runs need no flag to be remembered."""
    assert benchmark.DEFAULT_TIMEOUT_MULTIPLIER == 3.0


@pytest.mark.parametrize("multiplier", [1.0, 3.0, 5.0])
def test_a_budget_that_clears_the_scaled_clock_is_accepted(tmp_path, multiplier):
    manifest = manifest_with_budget(
        tmp_path, int(benchmark.MAX_TASK_TIMEOUT_SECONDS * multiplier)
    )
    benchmark.check_budget_clears_clock(manifest, multiplier)  # does not raise


def test_a_multiplier_the_harness_would_silently_undo_is_refused(tmp_path):
    """The failure this prevents is invisible: a timeout the agent never hit.

    Harbor would allow the longest task 36000s while the harness's own wait_for
    killed it at 12000s and Harbor recorded AgentTimeoutError — identical in the
    results to an agent that genuinely ran out of clock.
    """
    manifest = manifest_with_budget(tmp_path, benchmark.MAX_TASK_TIMEOUT_SECONDS)
    with pytest.raises(benchmark.DatasetError) as caught:
        benchmark.check_budget_clears_clock(manifest, 3.0)
    message = str(caught.value)
    # Actionable: names the file, the setting, and the number to put in it.
    assert "trial_budget.timeout_seconds" in message
    assert str(int(benchmark.MAX_TASK_TIMEOUT_SECONDS * 3)) in message
    assert manifest.name in message


def test_a_nonsense_multiplier_is_refused(tmp_path):
    with pytest.raises(benchmark.DatasetError, match="positive"):
        benchmark.check_budget_clears_clock(manifest_with_budget(tmp_path, 36000), 0)


def clock_argv(multiplier: float, tmp_path: Path) -> list[str]:
    args = Namespace(
        path=None,
        dataset=benchmark.DEFAULT_DATASET,
        include_task=[],
        exclude_task=[],
        resume_from=None,
        jobs_dir=tmp_path,
        attempts=1,
        manifest=Path("m.yaml"),
        endpoint_config=Path("e.json"),
        n_concurrent=1,
        timeout_multiplier=multiplier,
        job_name=None,
        upload=False,
        dry_run=False,
    )
    return benchmark.leaderboard_argv(args, tmp_path / "p.json", tmp_path / "bin")


def test_the_multiplier_reaches_harbor(tmp_path):
    argv = clock_argv(3.0, tmp_path)
    assert "--timeout-multiplier" in argv
    assert argv[argv.index("--timeout-multiplier") + 1] == "3.0"


def test_a_submittable_run_does_not_carry_the_flag_at_all(tmp_path):
    """Harbor rejects the knob being *set*, not just being non-unity.

    ``_check_no_job_overrides`` fails on ``agent_timeout_multiplier is not None``,
    so passing an explicit 1.0 would break submission just as surely as 3.0.
    """
    assert "--timeout-multiplier" not in clock_argv(1.0, tmp_path)
