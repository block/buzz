#!/usr/bin/env python3
"""Aggregate finished Harbor jobs into one row per benchmark condition.

Read-only: it walks a jobs directory and reports. Nothing is re-run, nothing is
mutated, and it is safe to point at a sweep that is still in progress.

The unit of grouping is ``manifest_sha256``, not the job directory name or even
the condition string. The hash covers the roster, the personas, the generation
config, and the prices — everything a condition *is* — so two runs pool only if
they were genuinely the same experiment. Edit a persona and forget to rename the
job and the hash changes, which shows up as two rows instead of one silently
wrong row.

Four things are reported next to the score rather than folded into it, because
each one can invalidate a comparison on its own:

* **Timeouts.** A stalled team burns its full trial budget and scores zero, so a
  timeout is penalised twice over. Pooling it into a mean hides the mechanism —
  "the 3-agent condition scored worse" reads very differently from "the 3-agent
  condition deadlocked a third of the time".
* **Launch failures.** The agent never started, so the trial measures the
  harness, not the model. These are never the condition's fault and always mean
  something needs fixing before the run counts.
* **Round-capped turns.** ``BUZZ_AGENT_MAX_ROUNDS`` ends a turn silently: the
  agent stops mid-task and publishes nothing. If any condition shows these, its
  comparison is void and the cap has to go up.
* **Unreconciled accounting.** A trial whose token counters never parsed reports
  a cost of zero. Averaging that in understates the condition's real cost.

Failed and timed-out trials count as reward 0 and their cost still counts. They
happened, they were paid for, and dropping them would bias every mean toward
whatever the condition happened to be good at.

Getting that right takes more than reading ``agent_result``. A trial that dies
before the agent publishes anything — a launch failure, a hard timeout — carries
no agent metadata at all, so the condition it belongs to cannot be read from the
place every healthy trial records it. Identifying such a trial by its own
``config.json`` instead is what keeps the worst outcomes in the denominator; the
first version of this script keyed on the metadata alone and silently dropped 7
of 15 trials, every one of them a failure. A summary that quietly discards
failures is worse than no summary, because it reads as a clean result.
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from dataclasses import dataclass, field
from datetime import datetime
from functools import lru_cache
from pathlib import Path
from statistics import mean
from typing import Any

from harbor_buzz_orchestra.verifier_health import verifier_broken

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_JOBS_DIR = PACKAGE_ROOT / "jobs"
# buzz-agent's stop reason when a turn exhausts BUZZ_AGENT_MAX_ROUNDS
# (crates/buzz-agent/src/types.rs:188). The turn ends without publishing, so
# this string in a log is the only trace the run leaves behind.
ROUND_CAP_MARKER = "max_turn_requests"
LOG_SUFFIXES = (".stdout.log", ".stderr.log")


@dataclass(slots=True)
class Trial:
    """One trial, reduced to the fields a comparison actually turns on."""

    condition: str
    manifest_sha256: str
    task: str
    reward: float
    input_tokens: int
    output_tokens: int
    cost_usd: float
    agent_seconds: float
    total_seconds: float
    outcome: str  # "ok" | "timeout" | "launch" | "error"
    reconciled: bool
    round_capped: bool
    verifier_broken: bool
    stopped_without_done: bool
    timeout_multiplier: float
    agents: int
    path: Path


@dataclass(slots=True)
class Condition:
    """Every trial that ran under one immutable manifest hash."""

    condition: str
    manifest_sha256: str
    trials: list[Trial] = field(default_factory=list)

    @property
    def n(self) -> int:
        return len(self.trials)

    @property
    def pass_rate(self) -> float:
        # Errors and timeouts are zeros, not exclusions — see module docstring.
        return mean(trial.reward for trial in self.trials)

    @property
    def timeouts(self) -> int:
        return sum(trial.outcome == "timeout" for trial in self.trials)

    @property
    def errors(self) -> int:
        return sum(trial.outcome == "error" for trial in self.trials)

    @property
    def launch_failures(self) -> int:
        return sum(trial.outcome == "launch" for trial in self.trials)

    @property
    def round_capped(self) -> int:
        return sum(trial.round_capped for trial in self.trials)

    @property
    def unreconciled(self) -> int:
        return sum(not trial.reconciled for trial in self.trials)

    @property
    def verifier_broken(self) -> int:
        return sum(trial.verifier_broken for trial in self.trials)

    @property
    def stopped_without_done(self) -> int:
        return sum(trial.stopped_without_done for trial in self.trials)

    @property
    def timeout_multipliers(self) -> list[float]:
        """Every distinct clock the trials in this row ran against.

        A list, not a scalar, because a resumed sweep can straddle a settings
        change, and a row that silently averages two different clocks is worse
        than one that admits it.
        """
        return sorted({trial.timeout_multiplier for trial in self.trials})

    @property
    def cost_per_trial(self) -> float:
        return mean(trial.cost_usd for trial in self.trials)

    @property
    def total_cost(self) -> float:
        return sum(trial.cost_usd for trial in self.trials)

    @property
    def tokens_per_trial(self) -> float:
        return mean(
            trial.input_tokens + trial.output_tokens for trial in self.trials
        )

    @property
    def seconds_per_trial(self) -> float:
        return mean(trial.agent_seconds for trial in self.trials)

    @property
    def agents(self) -> int:
        return max((trial.agents for trial in self.trials), default=0)


def _elapsed(block: object) -> float:
    """Seconds between a Harbor phase's start and finish stamps."""
    if not isinstance(block, dict):
        return 0.0
    started, finished = block.get("started_at"), block.get("finished_at")
    if not isinstance(started, str) or not isinstance(finished, str):
        return 0.0
    try:
        start = datetime.fromisoformat(started.replace("Z", "+00:00"))
        end = datetime.fromisoformat(finished.replace("Z", "+00:00"))
    except ValueError:
        return 0.0
    return max(0.0, (end - start).total_seconds())


def _classify(exception_info: object) -> str:
    """Separate a stall from a crash — they mean different things about a team.

    A launch failure gets its own bucket because it says nothing about the
    condition: the agent process never came up, so no model was ever asked to
    do the task. Reading it as a capability result would be a straight error.
    """
    if not exception_info:
        return "ok"
    text = json.dumps(exception_info).lower()
    if "timeout" in text:
        return "timeout"
    if "runtimelauncherror" in text or "failed to start" in text:
        return "launch"
    return "error"


def _round_capped(trial_dir: Path) -> bool:
    """Whether any agent's turn ended by exhausting its round budget.

    Grepped from the agent logs because nothing else records it: the turn ends
    with no published message, so from the channel's point of view the agent
    simply went quiet.
    """
    bundle = trial_dir / "agent" / "buzz"
    if not bundle.is_dir():
        return False
    for path in bundle.iterdir():
        if not path.name.endswith(LOG_SUFFIXES):
            continue
        try:
            if ROUND_CAP_MARKER in path.read_text(encoding="utf-8", errors="replace"):
                return True
        except OSError:
            continue
    return False


@lru_cache(maxsize=None)
def _manifest_identity(manifest_path: str) -> tuple[str, str] | None:
    """``(condition, sha256)`` read straight from a manifest on disk.

    The fallback for trials that never got far enough to record their own. The
    hash is recomputed from the manifest's canonical bytes, which is the same
    value a healthy trial reports — so a failed trial pools with its siblings
    rather than forming a phantom condition of its own.
    """
    candidate = Path(manifest_path)
    if not candidate.is_absolute():
        candidate = PACKAGE_ROOT / candidate
    try:
        from harbor_buzz_orchestra.manifest import ExperimentManifest

        manifest = ExperimentManifest.load(candidate)
    except Exception:  # noqa: BLE001 — an unreadable manifest is not fatal here
        return None
    return manifest.condition, manifest.sha256


def _timeout_multiplier(trial_dir: Path) -> float:
    """How much of Harbor's clock this trial was given, relative to the task's own.

    Read per trial rather than taken on trust, because it is the one setting that
    changes what a score *means* without changing anything about the agent: a
    condition run at 3x is answering "can it solve this at all", and one run at
    1.0 is answering "can it solve this inside Terminal-Bench's clock". Reporting
    the two in the same column without saying which is which would be the most
    misleading thing this script could do.

    Harbor writes both the job-wide multiplier and the agent-phase override into
    every trial's config.json. The agent-phase value wins when set, matching
    _resolve_timeout_sec in harbor/trial/trial.py.
    """
    try:
        config = json.loads((trial_dir / "config.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return 1.0
    for key in ("agent_timeout_multiplier", "timeout_multiplier"):
        value = config.get(key)
        if isinstance(value, (int, float)):
            return float(value)
    return 1.0


def _identity_from_config(trial_dir: Path) -> tuple[str, str] | None:
    """Which condition a trial belonged to, for trials that died before saying.

    Harbor writes the trial's own ``config.json`` before the agent runs, and it
    carries the manifest path this harness was invoked with. That makes it the
    one record of a launch failure's identity — and the presence of that kwarg
    is also what marks the trial as ours, so a run from a different agent still
    gets left alone.
    """
    try:
        config = json.loads((trial_dir / "config.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    kwargs = ((config.get("agent") or {}).get("kwargs") or {}) or {}
    manifest_path = kwargs.get("manifest")
    if not isinstance(manifest_path, str) or not manifest_path:
        return None
    return _manifest_identity(manifest_path)


def _bundle_summary(trial_dir: Path) -> dict[str, Any]:
    """The harness's own accounting for a trial, written during teardown.

    This survives outcomes Harbor's ``agent_result`` does not. The bundle is
    written from a ``finally`` block, so a trial that timed out still has its
    real token counts and price on disk even though Harbor recorded the trial
    as an exception with no agent result at all. Ignoring it would report the
    single most expensive class of trial — the one that ran the full budget
    and then failed — as costing nothing.
    """
    try:
        raw = (trial_dir / "agent" / "buzz" / "summary.json").read_text(
            encoding="utf-8"
        )
    except OSError:
        return {}
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        return {}
    return data if isinstance(data, dict) else {}


def read_trial(result_path: Path) -> Trial | None:
    """Reduce one trial's ``result.json``, or None if it is not a Buzz trial."""
    try:
        data = json.loads(result_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    trial_dir = result_path.parent
    agent_result = data.get("agent_result") or {}
    metadata = agent_result.get("metadata") or {}
    condition = metadata.get("condition")
    manifest_sha256 = metadata.get("manifest_sha256", "")
    tokens = (
        int(agent_result.get("n_input_tokens") or 0),
        int(agent_result.get("n_output_tokens") or 0),
    )
    cost = float(agent_result.get("cost_usd") or 0.0)
    reconciled = bool(metadata.get("accounting_reconciled"))
    agents = len(metadata.get("per_agent_tokens") or {})

    if not condition:
        # No agent metadata means Harbor never got a result — a launch failure
        # or a hard timeout. Those are the rows that matter most, so recover
        # what the harness wrote itself instead of dropping them.
        summary = _bundle_summary(trial_dir)
        condition = summary.get("condition")
        manifest_sha256 = summary.get("manifest_sha256", "")
        totals = summary.get("totals") or {}
        tokens = (
            int(totals.get("input_tokens") or 0),
            int(totals.get("output_tokens") or 0),
        )
        cost = float(totals.get("cost_usd") or 0.0)
        reconciled = bool((summary.get("accounting") or {}).get("reconciled"))
        agents = len(summary.get("roster") or ())
        if not condition:
            # No bundle either: the trial died before teardown could write one.
            # Its config still names the manifest, which is enough to keep the
            # row in the denominator even with nothing else to report.
            identity = _identity_from_config(trial_dir)
            if identity is None:
                return None  # not produced by this harness; leave it alone
            condition, manifest_sha256 = identity

    rewards = (data.get("verifier_result") or {}).get("rewards") or {}
    reward = rewards.get("reward")
    return Trial(
        condition=condition,
        manifest_sha256=manifest_sha256,
        task=data.get("task_name", "?"),
        # A trial with no reward file did not pass, whatever went wrong.
        reward=float(reward) if isinstance(reward, (int, float)) else 0.0,
        input_tokens=tokens[0],
        output_tokens=tokens[1],
        cost_usd=cost,
        agent_seconds=_elapsed(data.get("agent_execution")),
        total_seconds=_elapsed(data),
        outcome=_classify(data.get("exception_info")),
        reconciled=reconciled,
        round_capped=_round_capped(trial_dir),
        verifier_broken=verifier_broken(trial_dir),
        stopped_without_done=bool(metadata.get("stopped_without_done")),
        timeout_multiplier=_timeout_multiplier(trial_dir),
        agents=agents,
        path=trial_dir,
    )


def collect(jobs_dir: Path) -> list[Condition]:
    """Group every trial under ``jobs_dir`` by its manifest hash."""
    grouped: dict[tuple[str, str], Condition] = {}
    # */*/result.json is the trial level; */result.json is the job aggregate,
    # which read_trial rejects for having no condition metadata.
    for result_path in sorted(jobs_dir.glob("*/*/result.json")):
        trial = read_trial(result_path)
        if trial is None:
            continue
        key = (trial.condition, trial.manifest_sha256)
        grouped.setdefault(key, Condition(*key)).trials.append(trial)
    return sorted(grouped.values(), key=lambda c: (c.condition, c.manifest_sha256))


def format_table(conditions: list[Condition]) -> str:
    """The comparison table, with every caveat visible in its own column."""
    header = (
        f"{'condition':<26} {'hash':<8} {'ag':>2} {'n':>4} {'pass':>6} "
        f"{'$/trial':>8} {'$ total':>8} {'ktok':>7} {'sec':>6}  flags"
    )
    lines = [header, "-" * len(header)]
    for c in conditions:
        flags = []
        if c.timeouts:
            flags.append(f"timeout×{c.timeouts}")
        if c.launch_failures:
            flags.append(f"LAUNCH-FAILED×{c.launch_failures}")
        if c.errors:
            flags.append(f"error×{c.errors}")
        if c.round_capped:
            flags.append(f"ROUND-CAPPED×{c.round_capped}")
        if c.unreconciled:
            flags.append(f"unpriced×{c.unreconciled}")
        if c.verifier_broken:
            flags.append(f"VERIFIER-BROKEN×{c.verifier_broken}")
        if c.stopped_without_done:
            flags.append(f"no-done×{c.stopped_without_done}")
        for multiplier in c.timeout_multipliers:
            if multiplier != 1.0:
                flags.append(f"clock×{multiplier:g}")
        lines.append(
            f"{c.condition[:26]:<26} {c.manifest_sha256[:8]:<8} {c.agents:>2} "
            f"{c.n:>4} {c.pass_rate:>6.3f} {c.cost_per_trial:>8.4f} "
            f"{c.total_cost:>8.2f} {c.tokens_per_trial / 1000:>7.1f} "
            f"{c.seconds_per_trial:>6.1f}  {', '.join(flags)}"
        )
    if any(c.launch_failures for c in conditions):
        lines += [
            "",
            "WARNING: a launch failure means the agent never started, so the "
            "task was scored without a model ever attempting it. Fix the cause "
            "and re-run — these rows understate the condition's real score.",
        ]
    if any(c.verifier_broken for c in conditions):
        lines += [
            "",
            "WARNING: a VERIFIER-BROKEN trial could not install its own tests, so "
            "its reward of 0 says nothing about the agent — the score was never "
            "computed. The usual cause is a TLS-intercepting proxy between the "
            "container and PyPI; benchmark.py now refuses to start under one. "
            "Any flagged condition must be re-run, not reported.",
        ]
    if any(m != 1.0 for c in conditions for m in c.timeout_multipliers):
        lines += [
            "",
            "NOTE: a clock×N row ran with every Harbor phase deadline multiplied "
            "by N, so its score answers 'can the team solve this at all' rather "
            "than 'inside Terminal-Bench's clock'. Not comparable to published "
            "leaderboard numbers, and Harbor will refuse it as a submission. "
            "Quote the multiplier wherever the score is quoted; the sec column "
            "is what the extra time actually cost.",
        ]
    if any(len(c.timeout_multipliers) > 1 for c in conditions):
        lines += [
            "",
            "WARNING: a row above pools trials run under different clocks, which "
            "is not one condition. Split the job dirs or re-run the odd half.",
        ]
    if any(c.stopped_without_done for c in conditions):
        lines += [
            "",
            "NOTE: a no-done trial ended its turn without posting DONE. The "
            "reward is real — the verifier scored the container either way — but "
            "the completion protocol was dropped, so the agent got no chance to "
            "check its own work. Compare the flag across conditions: a rate that "
            "moves with team shape is a finding, not noise.",
        ]
    if any(c.round_capped for c in conditions):
        lines += [
            "",
            "WARNING: a round-capped turn ends silently mid-task. Any condition "
            "flagged ROUND-CAPPED is not comparable — raise BUZZ_AGENT_MAX_ROUNDS "
            "and re-run it.",
        ]
    if any(c.unreconciled for c in conditions):
        lines += [
            "",
            "WARNING: unpriced trials reported no usage, so their cost counts as "
            "$0 and this condition's cost is understated.",
        ]
    return "\n".join(lines)


def write_csv(conditions: list[Condition], path: Path) -> None:
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                "condition", "manifest_sha256", "agents", "n", "pass_rate",
                "cost_usd_per_trial", "cost_usd_total", "tokens_per_trial",
                "agent_seconds_per_trial", "timeouts", "launch_failures",
                "errors", "round_capped", "unreconciled", "verifier_broken",
                "stopped_without_done", "timeout_multipliers",
            ]
        )
        for c in conditions:
            writer.writerow(
                [
                    c.condition, c.manifest_sha256, c.agents, c.n,
                    f"{c.pass_rate:.6f}", f"{c.cost_per_trial:.6f}",
                    f"{c.total_cost:.6f}", f"{c.tokens_per_trial:.1f}",
                    f"{c.seconds_per_trial:.2f}", c.timeouts,
                    c.launch_failures, c.errors,
                    c.round_capped, c.unreconciled, c.verifier_broken,
                    c.stopped_without_done,
                    " ".join(f"{m:g}" for m in c.timeout_multipliers),
                ]
            )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "jobs_dir", nargs="?", type=Path, default=DEFAULT_JOBS_DIR,
        help=f"Job output root (default: {DEFAULT_JOBS_DIR})",
    )
    parser.add_argument("--csv", type=Path, help="Also write the table as CSV")
    parser.add_argument(
        "--json", action="store_true", help="Emit per-trial rows as JSON instead"
    )
    parser.add_argument(
        "--failures", action="store_true",
        help="List the trials that scored zero, with their paths",
    )
    args = parser.parse_args(argv)

    if not args.jobs_dir.is_dir():
        print(f"no such jobs directory: {args.jobs_dir}", file=sys.stderr)
        return 2
    conditions = collect(args.jobs_dir)
    if not conditions:
        print(f"no Buzz trials found under {args.jobs_dir}", file=sys.stderr)
        return 1

    if args.json:
        print(
            json.dumps(
                [
                    {
                        **{
                            key: getattr(trial, key)
                            for key in Trial.__slots__
                            if key != "path"
                        },
                        "path": str(trial.path),
                    }
                    for condition in conditions
                    for trial in condition.trials
                ],
                indent=2,
            )
        )
        return 0

    print(format_table(conditions))
    if args.csv:
        write_csv(conditions, args.csv)
        print(f"\ncsv: {args.csv}")
    if args.failures:
        print("\nzero-scoring trials:")
        for condition in conditions:
            for trial in condition.trials:
                if trial.reward == 0.0:
                    print(
                        f"  {condition.condition:<24} {trial.task:<44} "
                        f"{trial.outcome:<8} {trial.path}"
                    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
