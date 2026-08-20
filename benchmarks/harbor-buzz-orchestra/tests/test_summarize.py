import importlib.util
import json
import sys
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "summarize.py"
_spec = importlib.util.spec_from_file_location("summarize_under_test", _SCRIPT)
summarize = importlib.util.module_from_spec(_spec)
# Registered before exec: @dataclass(slots=True) rebuilds the class and looks it
# up by __module__, which fails if the module is not yet importable by name.
sys.modules[_spec.name] = summarize
_spec.loader.exec_module(summarize)


def write_trial(
    jobs_dir: Path,
    job: str,
    name: str,
    *,
    condition: str = "tb-solo-luna",
    manifest_sha256: str = "a" * 64,
    reward: float | None = 1.0,
    cost: float = 0.05,
    tokens: tuple[int, int] = (1000, 100),
    exception: object = None,
    reconciled: bool = True,
    agents: int = 1,
    log: str | None = None,
    verifier_stdout: str | None = None,
    timeout_multiplier: float | None = None,
) -> Path:
    """One trial directory shaped like Harbor's, with a Buzz bundle inside."""
    trial_dir = jobs_dir / job / name
    trial_dir.mkdir(parents=True)
    if verifier_stdout is not None:
        (trial_dir / "verifier").mkdir()
        (trial_dir / "verifier" / "test-stdout.txt").write_text(
            verifier_stdout, encoding="utf-8"
        )
    if log is not None:
        bundle = trial_dir / "agent" / "buzz"
        bundle.mkdir(parents=True)
        (bundle / "solo-1.stdout.log").write_text(log, encoding="utf-8")
    if timeout_multiplier is not None:
        (trial_dir / "config.json").write_text(
            json.dumps({"timeout_multiplier": timeout_multiplier}), encoding="utf-8"
        )
    (trial_dir / "result.json").write_text(
        json.dumps(
            {
                "task_name": f"terminal-bench/{name}",
                "started_at": "2026-07-27T00:00:00.000000Z",
                "finished_at": "2026-07-27T00:01:00.000000Z",
                "agent_execution": {
                    "started_at": "2026-07-27T00:00:10.000000Z",
                    "finished_at": "2026-07-27T00:00:40.000000Z",
                },
                "exception_info": exception,
                "verifier_result": (
                    {"rewards": {"reward": reward}} if reward is not None else {}
                ),
                "agent_result": {
                    "n_input_tokens": tokens[0],
                    "n_output_tokens": tokens[1],
                    "cost_usd": cost,
                    "metadata": {
                        "condition": condition,
                        "manifest_sha256": manifest_sha256,
                        "accounting_reconciled": reconciled,
                        "per_agent_tokens": {f"a{i}": {} for i in range(agents)},
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    return trial_dir


def test_trials_group_by_manifest_hash_not_by_job_name(tmp_path):
    """Two jobs of the same condition pool; an edited persona does not.

    The hash covers the personas, so editing one and re-running under the same
    condition name must surface as two rows. Pooling them would silently average
    a result across two different experiments.
    """
    write_trial(tmp_path, "run-one", "t1", manifest_sha256="a" * 64)
    write_trial(tmp_path, "run-two", "t2", manifest_sha256="a" * 64)
    write_trial(tmp_path, "run-three", "t3", manifest_sha256="b" * 64)

    conditions = summarize.collect(tmp_path)
    assert [(c.manifest_sha256[:1], c.n) for c in conditions] == [("a", 2), ("b", 1)]


def test_timeouts_score_zero_and_are_reported_separately(tmp_path):
    """A stall is a result, not a missing data point — but it is its own column.

    It costs the full trial budget and scores zero, so it is penalised twice.
    Folding it into the mean hides that the mechanism was a deadlock.
    """
    write_trial(tmp_path, "job", "pass", reward=1.0)
    write_trial(
        tmp_path, "job", "stalled", reward=None, cost=0.40,
        exception={"exc_type": "TimeoutError", "message": "trial budget exceeded"},
    )

    (condition,) = summarize.collect(tmp_path)
    assert condition.n == 2
    assert condition.pass_rate == 0.5
    assert condition.timeouts == 1
    assert condition.errors == 0
    # The stall's cost still counts: it was paid for.
    assert condition.total_cost == 0.45


def test_a_crash_a_stall_and_a_failed_launch_are_three_different_things(tmp_path):
    """Each says something different about where the fault lies.

    A stall is the team deadlocking, a crash is the harness breaking mid-run,
    and a failed launch means no model was ever asked to do the task. Only the
    first is a property of the condition being measured.
    """
    write_trial(
        tmp_path, "job", "never-started", reward=None,
        exception={"exc_type": "RuntimeLaunchError", "message": "agent exited"},
    )
    write_trial(
        tmp_path, "job", "crashed", reward=None,
        exception={"exc_type": "DockerException", "message": "no such container"},
    )
    write_trial(
        tmp_path, "job", "stalled", reward=None,
        exception={"exc_type": "AgentTimeoutError", "message": "budget exceeded"},
    )
    (condition,) = summarize.collect(tmp_path)
    assert (condition.launch_failures, condition.errors, condition.timeouts) == (
        1,
        1,
        1,
    )


def test_a_round_capped_turn_is_detected_from_the_agent_log(tmp_path):
    """The cap ends a turn silently, so the log is the only evidence it happened.

    A condition that hits it is not comparable — the agent stopped mid-task for a
    reason unrelated to its ability to do the task.
    """
    write_trial(
        tmp_path, "job", "capped",
        log="INFO buzz_acp: turn finished stop_reason=max_turn_requests\n",
    )
    (condition,) = summarize.collect(tmp_path)
    assert condition.round_capped == 1
    assert "ROUND-CAPPED" in summarize.format_table([condition])


def test_a_verifier_that_could_not_install_its_tests_is_not_a_failed_agent(tmp_path):
    """Reward 0 with no verifier is not the same result as reward 0 with one.

    A TLS-intercepting proxy between the container and PyPI kills the verifier's
    own `pip install pytest`. Harbor still records reward 0, so an entire sweep
    can summarise as a clean 0% while every agent did its job — which is exactly
    what happened to one 89-task run. The flag is what makes the two legible.
    """
    write_trial(
        tmp_path, "job", "uv-tls", reward=0.0,
        verifier_stdout=(
            "error: Failed to fetch: `https://files.pythonhosted.org/...`\n"
            "  Caused by: invalid peer certificate: UnknownIssuer\n"
        ),
    )
    write_trial(
        tmp_path, "job", "pip-tls", reward=0.0,
        verifier_stdout=(
            "SSLError(SSLCertVerificationError(1, '[SSL: CERTIFICATE_VERIFY_FAILED] "
            "certificate verify failed: self-signed certificate in certificate "
            "chain'))\n/tests/test.sh: line 6: pytest: command not found\n"
        ),
    )
    # The control: a verifier that ran and the agent simply got it wrong.
    write_trial(
        tmp_path, "job", "honest-miss", reward=0.0,
        verifier_stdout="collected 3 items\n3 failed\n",
    )
    (condition,) = summarize.collect(tmp_path)
    assert condition.verifier_broken == 2
    table = summarize.format_table([condition])
    assert "VERIFIER-BROKEN×2" in table
    assert "must be re-run, not reported" in table


def test_an_empty_apt_index_is_a_broken_verifier_not_a_wrong_answer(tmp_path):
    """`Unable to locate package` means apt-get update failed, not that the model did.

    The verifier installs its own test dependencies (`apt-get install -y curl
    binutils`, then uv, then pytest). When its `apt-get update` cannot validate
    a certificate the index is empty, so apt cannot find packages that plainly
    exist and the trial is recorded as reward 0.0. Three A1 trials failed this
    way. container_runtime's SYSTEM_CA_BUNDLE seed removes the usual cause;
    this marker is the backstop for whatever it does not cover.
    """
    write_trial(
        tmp_path, "job", "apt-index-empty", reward=0.0,
        verifier_stdout=(
            "Err:1 https://archive.ubuntu.com/ubuntu noble InRelease\n"
            "  Certificate verification failed: The certificate is NOT trusted.\n"
            "E: Unable to locate package curl\n"
            "E: Unable to locate package binutils\n"
        ),
    )
    # The control that matters for this marker: a task whose own output
    # mentions packages must not be mistaken for a broken verifier.
    write_trial(
        tmp_path, "job", "honest-miss", reward=0.0,
        verifier_stdout="collected 3 items\n1 failed, 2 passed\n",
    )
    (condition,) = summarize.collect(tmp_path)
    assert condition.verifier_broken == 1


def test_a_missing_verifier_log_is_not_evidence_of_anything(tmp_path):
    """Absence of the file must not be read as a broken verifier.

    Launch failures never produce one, and flagging them here would double-count
    a row LAUNCH-FAILED already explains.
    """
    write_trial(tmp_path, "job", "no-verifier-dir", reward=0.0)
    (condition,) = summarize.collect(tmp_path)
    assert condition.verifier_broken == 0
    assert "VERIFIER-BROKEN" not in summarize.format_table([condition])


def test_unpriced_trials_are_flagged_because_they_understate_cost(tmp_path):
    write_trial(tmp_path, "job", "priced", cost=0.10, reconciled=True)
    write_trial(tmp_path, "job", "unpriced", cost=0.0, reconciled=False)
    (condition,) = summarize.collect(tmp_path)
    assert condition.unreconciled == 1
    assert condition.cost_per_trial == 0.05  # dragged down by the zero
    assert "unpriced" in summarize.format_table([condition])


def test_job_level_results_are_not_mistaken_for_trials(tmp_path):
    """``jobs/<job>/result.json`` is an aggregate and must not be counted again."""
    write_trial(tmp_path, "job", "t1")
    (tmp_path / "job" / "result.json").write_text(
        json.dumps({"n_total_trials": 1, "stats": {}}), encoding="utf-8"
    )
    (condition,) = summarize.collect(tmp_path)
    assert condition.n == 1


def test_non_buzz_trials_are_left_alone(tmp_path):
    """A jobs dir may hold runs from another agent; they are not ours to report."""
    other = tmp_path / "job" / "trial"
    other.mkdir(parents=True)
    (other / "result.json").write_text(
        json.dumps({"agent_result": {"metadata": {}}}), encoding="utf-8"
    )
    assert summarize.collect(tmp_path) == []


def write_manifest(path: Path, condition: str = "tb-solo-luna") -> str:
    """A minimal valid manifest, returned with its canonical hash."""
    path.write_text(
        "\n".join(
            [
                'schema_version: "1"',
                f"condition: {condition}",
                "roster:",
                "  - id: solo",
                "    kind: orchestrator",
                "    role: solo",
                "    count: 1",
                "    endpoint: gpt-5.6-luna",
                "    model_revision: gpt-5.6-luna",
                "    prompt:",
                "      path: personas/bench/solo.md",
                f'      sha256: "{"0" * 64}"',
                "prices:",
                "  gpt-5.6-luna:",
                "    input_per_million_usd: 1.0",
                "    cached_input_per_million_usd: 0.1",
                "    output_per_million_usd: 6.0",
                "    cache_read_rate: 0.0",
                "trial_budget:",
                "  timeout_seconds: 900",
                "",
            ]
        ),
        encoding="utf-8",
    )
    from harbor_buzz_orchestra.manifest import ExperimentManifest

    return ExperimentManifest.load(path).sha256


def test_a_timed_out_trials_real_cost_comes_from_the_bundle(tmp_path):
    """Harbor reports no agent result for a stall; the harness's bundle does.

    A trial that ran the full budget and then failed is the most expensive kind
    there is — the first live one cost $1.31 — and Harbor records it with
    ``agent_result: null``. Trusting that alone prices exactly the wrong trials
    at zero, which flatters every condition that stalls.
    """
    manifest = tmp_path / "m.yaml"
    digest = write_manifest(manifest)
    trial_dir = write_dead_trial(
        tmp_path, "job", "stalled", manifest,
        {"exception_type": "AgentTimeoutError", "exception_message": "timeout"},
    )
    bundle = trial_dir / "agent" / "buzz"
    bundle.mkdir(parents=True)
    (bundle / "summary.json").write_text(
        json.dumps(
            {
                "condition": "tb-solo-luna",
                "manifest_sha256": digest,
                "accounting": {"reconciled": True},
                "roster": [{"agent_id": "solo-1"}],
                "totals": {
                    "input_tokens": 1099599,
                    "output_tokens": 35001,
                    "cost_usd": 1.309605,
                },
            }
        ),
        encoding="utf-8",
    )
    (condition,) = summarize.collect(tmp_path)
    assert condition.total_cost == pytest.approx(1.309605)
    assert condition.tokens_per_trial == pytest.approx(1134600)
    assert condition.timeouts == 1
    # It reported usage, so it is priced — not an unpriced-trial warning.
    assert condition.unreconciled == 0
    assert condition.agents == 1


def write_dead_trial(jobs_dir: Path, job: str, name: str, manifest: Path, exc: dict):
    """A trial that died before the agent recorded any metadata.

    This is what a launch failure and a hard timeout both look like on disk:
    ``agent_result`` is absent entirely, so the condition can only come from
    the config Harbor wrote before the agent ran.
    """
    trial_dir = jobs_dir / job / name
    trial_dir.mkdir(parents=True)
    (trial_dir / "config.json").write_text(
        json.dumps({"agent": {"kwargs": {"manifest": str(manifest)}}}),
        encoding="utf-8",
    )
    (trial_dir / "result.json").write_text(
        json.dumps(
            {
                "task_name": f"terminal-bench/{name}",
                "started_at": "2026-07-27T00:00:00.000000Z",
                "finished_at": "2026-07-27T00:15:00.000000Z",
                "exception_info": exc,
                "verifier_result": {},
                "agent_result": None,
            }
        ),
        encoding="utf-8",
    )
    return trial_dir


def test_a_trial_that_died_before_reporting_is_still_counted(tmp_path):
    """The failures that matter most carry no agent metadata at all.

    Keying the condition off ``agent_result.metadata`` alone drops exactly the
    rows a summary exists to surface — a launch failure and a hard timeout both
    write no metadata — and the run then reads as a clean pass rate over
    whatever happened to survive. Recovering identity from the trial's own
    config is what keeps them in the denominator.
    """
    manifest = tmp_path / "m.yaml"
    digest = write_manifest(manifest)
    write_trial(
        tmp_path, "job", "healthy",
        condition="tb-solo-luna", manifest_sha256=digest, reward=1.0,
    )
    write_dead_trial(
        tmp_path, "job", "never-started", manifest,
        {"exception_type": "RuntimeLaunchError",
         "exception_message": "agent processes exited early: ['solo-1']"},
    )
    write_dead_trial(
        tmp_path, "job", "stalled", manifest,
        {"exception_type": "AgentTimeoutError", "exception_message": "timeout"},
    )

    # One condition, not three: the recovered hash matches the healthy trial's.
    (condition,) = summarize.collect(tmp_path)
    assert condition.n == 3
    assert condition.pass_rate == pytest.approx(1 / 3)
    assert condition.launch_failures == 1
    assert condition.timeouts == 1
    # A launch failure is harness breakage, never a capability result.
    assert condition.errors == 0
    table = summarize.format_table([condition])
    assert "LAUNCH-FAILED×1" in table
    assert "never started" in table


def test_a_condition_whose_every_trial_died_still_appears(tmp_path):
    """The all-failed case is the one a summary must never render as absent."""
    manifest = tmp_path / "m.yaml"
    digest = write_manifest(manifest)
    write_dead_trial(
        tmp_path, "job", "dead", manifest,
        {"exception_type": "RuntimeLaunchError", "exception_message": "no start"},
    )
    (condition,) = summarize.collect(tmp_path)
    assert (condition.condition, condition.manifest_sha256) == (
        "tb-solo-luna",
        digest,
    )
    assert (condition.n, condition.pass_rate) == (1, 0.0)


def test_a_dead_trial_without_a_manifest_reference_is_not_claimed(tmp_path):
    """Recovery must not sweep in another agent's failures."""
    trial_dir = tmp_path / "job" / "foreign"
    trial_dir.mkdir(parents=True)
    (trial_dir / "config.json").write_text(
        json.dumps({"agent": {"name": "some-other-agent", "kwargs": {}}}),
        encoding="utf-8",
    )
    (trial_dir / "result.json").write_text(
        json.dumps({"exception_info": {"exception_type": "RuntimeLaunchError"}}),
        encoding="utf-8",
    )
    assert summarize.collect(tmp_path) == []


def test_csv_round_trips_every_condition(tmp_path):
    write_trial(tmp_path, "job", "t1", condition="A1", manifest_sha256="a" * 64)
    write_trial(tmp_path, "job", "t2", condition="B1", manifest_sha256="b" * 64)
    out = tmp_path / "out.csv"
    summarize.write_csv(summarize.collect(tmp_path), out)
    rows = out.read_text(encoding="utf-8").splitlines()
    assert rows[0].startswith("condition,manifest_sha256,agents,n,pass_rate")
    assert len(rows) == 3


def test_a_stretched_clock_is_never_reported_as_a_leaderboard_score(tmp_path):
    """The one setting that changes what a score means without touching the agent.

    A 3x row answers "can the team solve this at all"; a 1.0 row answers "inside
    Terminal-Bench's clock". Printing them in the same column with nothing to
    tell them apart is the most misleading thing this script could do.
    """
    write_trial(tmp_path, "job", "t1", timeout_multiplier=3.0)
    table = summarize.format_table(summarize.collect(tmp_path))
    assert "clock×3" in table
    assert "not comparable to published" in table.lower()
    assert "refuse it as a submission" in table


def test_an_unstretched_run_says_nothing_about_clocks(tmp_path):
    write_trial(tmp_path, "job", "t1", timeout_multiplier=1.0)
    write_trial(tmp_path, "job", "t2")  # no config.json at all
    table = summarize.format_table(summarize.collect(tmp_path))
    assert "clock×" not in table


def test_pooling_two_different_clocks_into_one_row_is_called_out(tmp_path):
    """A resumed sweep can straddle a settings change; that is not one condition."""
    write_trial(tmp_path, "job", "before", timeout_multiplier=1.0)
    write_trial(tmp_path, "job", "after", timeout_multiplier=3.0)
    table = summarize.format_table(summarize.collect(tmp_path))
    assert "clock×3" in table
    assert "pools trials run under different clocks" in table
