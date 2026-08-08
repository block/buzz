#!/usr/bin/env python3
"""Teach an installed Harbor 0.16.x to honor LHTB's ``continue_until_timeout``.

Why this exists
---------------
30 of LHTB's 46 tasks set ``continue_until_timeout = true`` in ``[agent]``.
Stock Harbor drops the field on the floor (Pydantic ignores unknown keys), so
those tasks run single-shot: the agent emits ``DONE:``/``task_complete``, the
trial ends, and whatever happens to be on disk is scored. Our first full EC2
run showed exactly what that costs -- 24 of 24 trials self-reported success at
a mean of 7.8 minutes against a multi-hour budget, and scored 0 completions.

With the flag honored, the trial runs an interim verifier after the agent
stops; if the reward is short of 1.0 the agent is re-invoked in the same
container with the verifier feedback appended, until it passes or the agent
timeout expires.

Why a script and not ``git apply``
----------------------------------
The reference diff (``LHTB/harbor/patches/continue-until-timeout.patch``) is
written against Harbor 0.7.0 and does not apply to 0.16.1 -- the trial class
was split into ``Trial`` / ``SingleStepTrial`` / ``MultiStepTrial`` and the
agent invocation moved into ``Trial._run_agent_phase`` behind a stack of
context managers. More importantly the orchestra consumes Harbor as a *wheel*
from artifactory, not a checkout, so the thing that needs editing is
``site-packages`` on each benchmark box. This script finds that install,
applies both edits by exact-string replacement, and is safe to re-run.

Where the loop lives
--------------------
``SingleStepTrial._run_agent``, not ``Trial._run_agent_phase``. Patching the
phase helper would mean re-entering its network-policy / default-user / log
context managers per iteration and hand-merging the ``AgentContext`` it resets
on every call. Driving it from the caller keeps the loop readable and leaves
the phase helper's contract untouched. All 46 LHTB tasks are single-step with
a shared verifier, so ``MultiStepTrial`` is deliberately left alone.

PER-PHASE RECEIPTS -- broken 2026-07-30, FIXED 2026-07-30 (and the first diagnosis
here was wrong)
----------------------------------------------------------------------------------
Symptom: ``receipts.jsonl`` under-reported a multi-phase trial badly. Measured on
``lhtb46-team``: ``$1.76`` in receipts against ``$9.90`` in ``agent_result`` on
``grammar-fuzz-coverage-hunt`` (5.6x), and ``$0.03`` against ``$2.05`` on
``langchain-version-migration`` (63x).

**The first diagnosis written here blamed ``_sync_agent_output``, and it was
wrong.** That function is only ``_download_agent_logs()`` +
``_populate_agent_context()``; it does not touch receipts, and it could not be
made to -- ``_download_agent_logs`` early-returns on
``self._are_agent_logs_downloaded``, so calling it per phase (the fix originally
prescribed here) would have been a no-op after phase 1 and would have "fixed"
nothing while looking correct.

The actual writer is ``BuzzContainerRuntime.run()``, one level down. Its own
``finally`` calls ``bundle.write()`` -> ``accounting.write_receipts()``, which
opens ``receipts.jsonl`` with mode ``"w"``. ``run()`` is the agent invocation, so
it executes **once per phase**, truncating the same path every time: last phase
wins. The per-phase data was always being produced correctly and then thrown away
by the next phase, which is why the survivor is a plausible-looking small number
rather than an obviously broken one.

The fix, in the loop below: move each phase's file aside to
``receipts.phase-NN.jsonl`` as soon as that phase returns (in a ``finally``, since
the deadline phase raises *after* its receipts are written), then rebuild
``receipts.jsonl`` in the trial's ``finally`` as all phases concatenated.
Concatenating rather than only numbering matters -- every analysis tool reads
``receipts.jsonl`` and sums its rows, so deleting it would turn this into a
silent ``$0``, and summing rows per seat is already how those tools aggregate.
``agent_result.metadata`` also gains ``receipts_phase_files`` and
``receipts_cost_usd`` so the two independent accountings can be asserted equal
rather than chosen between.

**Runs made BEFORE this fix (the 46-task LHTB wave on h1/h2) still need the
inverted rule: score their cost and tokens from ``agent_result``.** Receipts there
hold one phase of many; the per-seat split is unrecoverable on those trials except
where a trial happened to run a single phase. On an *unpatched* run the original
rule still holds -- receipts are authoritative, because a timed-out trial has no
``agent_result`` at all. See docs/02 SS3.1 and docs/09.

Two things measured on the real h1/h2 trials while fixing this, both of which
contradict a guess that was easy to make here:

1. **Most trials leave this loop CLEANLY, with no exception.** The ``remaining <=
   0`` checks break the loop, so the last phase returned normally and its usage
   *was* merged. 4 of 5 sampled team trials had no ``exception_info`` at all
   despite running 15-194 phases. So ``agent_result`` really is complete on the
   common path, and the "agent_result is missing its last phase" worry applies
   only to the ``AgentTimeoutError`` minority.
2. **On that minority the final phase's usage exists NOWHERE, and receipts read
   $0.00 rather than missing.** ``epidemic-inverse-control-audit``: 79 phases,
   ``AgentTimeoutError``, ``agent_result`` $5.45, receipts $0.00 across 3 rows.
   ``BuzzContainerRuntime._settle_usage`` runs only on its success path, so a
   cancelled phase's tokens are never flushed before teardown. The loop below
   therefore refuses to merge a zero-cost recovered phase and records it in
   ``agent_result.metadata.unsettled_phases`` instead -- such a trial's cost is a
   floor. Do NOT reconstruct these as ``agent_result + receipts``; on that path
   the receipts are the zero, not the missing piece.

Not fixed, and deliberately out of scope: ``summary.json`` beside the receipts is
still last-phase-only, for the same truncating reason. Nothing in the analysis
path reads it. Nor is the unsettled-usage hole itself -- closing it means flushing
usage on the cancellation path inside ``BuzzContainerRuntime.run``, which is a
change to the runtime rather than to this patch.

Usage:  python3 apply_continue_until_timeout.py [--check] [--harbor-root DIR]
"""

from __future__ import annotations

import argparse
import importlib.util
import sys
from pathlib import Path

MARKER = "continue_until_timeout"

# --- edit 1: the config field -------------------------------------------------

CONFIG_ANCHOR = """class AgentConfig(PhaseNetworkPolicyConfig):
    timeout_sec: float | None = None
    user: str | int | None = Field(
        default=None,
        description="Username or UID to run the agent as. None uses the environment's default USER (e.g., root).",
    )
"""

CONFIG_PATCHED = (
    CONFIG_ANCHOR
    + """    continue_until_timeout: bool = Field(
        default=False,
        description=(
            "When true, the trial keeps restarting the agent after early "
            "completion or failed interim verification until agent.timeout_sec "
            "elapses or the verifier passes. An agent declaring itself done "
            "does not end the trial."
        ),
    )
"""
)

# --- edit 2: imports on single_step.py ---------------------------------------

IMPORT_ANCHOR = """from typing import override
import asyncio
"""

IMPORT_PATCHED = """from typing import override
import asyncio
import json
import time
from pathlib import Path

from harbor.models.agent.context import AgentContext
from harbor.models.verifier.result import VerifierResult
"""

# --- edit 3: helpers + the phase loop ----------------------------------------

HELPERS = '''

def _merge_agent_contexts(
    accumulated: "AgentContext | None", phase: "AgentContext"
) -> "AgentContext":
    """Sum per-phase agent metrics into one context.

    ``Trial._run_agent_phase`` assigns a fresh ``AgentContext`` to the target on
    every call, so without this each phase would clobber the last and a trial
    that ran six phases would report the tokens and cost of only the sixth.
    """
    if accumulated is None:
        return phase.model_copy(deep=True)

    def _sum_int(a: int | None, b: int | None) -> int | None:
        if a is None and b is None:
            return None
        return (a or 0) + (b or 0)

    def _sum_float(a: float | None, b: float | None) -> float | None:
        if a is None and b is None:
            return None
        return (a or 0.0) + (b or 0.0)

    merged_rollouts: list = []
    if accumulated.rollout_details:
        merged_rollouts.extend(accumulated.rollout_details)
    if phase.rollout_details:
        merged_rollouts.extend(phase.rollout_details)

    merged_metadata: dict = dict(accumulated.metadata or {})
    for key, value in (phase.metadata or {}).items():
        if key == "n_episodes" and isinstance(value, int):
            prior = merged_metadata.get("n_episodes")
            merged_metadata["n_episodes"] = (
                int(prior) + value if isinstance(prior, int) else value
            )
        else:
            merged_metadata[key] = value

    return AgentContext(
        n_input_tokens=_sum_int(accumulated.n_input_tokens, phase.n_input_tokens),
        n_cache_tokens=_sum_int(accumulated.n_cache_tokens, phase.n_cache_tokens),
        n_output_tokens=_sum_int(accumulated.n_output_tokens, phase.n_output_tokens),
        cost_usd=_sum_float(accumulated.cost_usd, phase.cost_usd),
        rollout_details=merged_rollouts or None,
        metadata=merged_metadata or None,
    )


def _verifier_passed(result: "VerifierResult | None") -> bool:
    if result is None or not result.rewards:
        return False
    try:
        return float(result.rewards.get("reward", 0)) >= 1.0
    except (TypeError, ValueError):
        return False


def _context_from_receipts(path: Path) -> "AgentContext | None":
    """Rebuild one phase's AgentContext from the receipts that phase wrote.

    Needed because a phase cancelled at the deadline never reports its usage.
    ``_run_agent_phase`` hands the agent a fresh empty ``AgentContext`` and
    ``BuzzOrchestraAgent.run`` copies totals into it only *after*
    ``runtime.run()`` returns -- and ``asyncio.wait_for`` cancels that coroutine
    on timeout, so the copy never happens. The spend is real and did get written,
    by ``BuzzContainerRuntime.run``'s own ``finally``, to that phase's receipts.
    Reading it back from there is the only way ``agent_result`` can include the
    phase that spent the budget, which for this loop is the EXPECTED ending.

    Returns None if there is nothing usable, never a zeroed context: a zero would
    merge in silently and read as "this phase was free".
    """
    try:
        rows = [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
    except Exception:
        return None
    if not rows:
        return None

    def total(key: str) -> int:
        return sum(int(row.get(key) or 0) for row in rows)

    return AgentContext(
        n_input_tokens=total("input_tokens"),
        n_cache_tokens=total("estimated_cache_read_tokens"),
        n_output_tokens=total("output_tokens"),
        cost_usd=sum(float(row.get("cost_usd") or 0.0) for row in rows),
    )


def _sum_receipt_cost(path: Path) -> float | None:
    """Total cost_usd across a receipts file, or None if it cannot be read.

    Used only to cross-check the rebuilt receipts against ``agent_result``.
    Returns None rather than 0.0 on any failure: a zero here would be
    indistinguishable from a genuinely free trial, and reading absent data as
    zero is the exact bug this whole patch section exists to undo.
    """
    try:
        total = 0.0
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            total += float(json.loads(line).get("cost_usd") or 0.0)
        return total
    except Exception:
        return None


def _read_verifier_feedback(verifier_dir: Path) -> str:
    """Best-effort human-readable reason the interim verifier was unhappy.

    LHTB verifiers emit graded partial credit, so the reward alone is real
    signal to the agent -- it says how close the last attempt got.
    """
    details_path = verifier_dir / "migration_details.json"
    if details_path.exists():
        try:
            data = json.loads(details_path.read_text())
            failure = data.get("failure")
            if isinstance(failure, str) and failure.strip():
                return failure.strip()
            return json.dumps(data, indent=2)[:4000]
        except Exception:
            pass

    reward_path = verifier_dir / "reward.txt"
    if reward_path.exists():
        try:
            return (
                f"Verifier reward so far: {reward_path.read_text().strip()} "
                "(1.0 is a full pass)."
            )
        except OSError:
            pass

    return "Verification did not fully pass."


def _format_continue_instruction(
    base_instruction: str, *, phase: int, remaining_sec: int, feedback: str
) -> str:
    return (
        f"{base_instruction.rstrip()}\\n\\n"
        "---\\n"
        "## VERIFICATION DID NOT PASS - CONTINUE WORKING\\n\\n"
        f"Your previous attempt did not fully pass verification (phase {phase}). "
        f"You have approximately {remaining_sec} seconds left in this trial.\\n\\n"
        "**Do not stop.** This trial ends only at timeout or full verifier "
        "success. Your shell session and files are exactly as you left them - "
        "keep improving the solution.\\n\\n"
        "Verifier feedback:\\n"
        f"{feedback.strip()}\\n"
    )

'''

RUN_AGENT_ANCHOR = """    async def _run_agent(self) -> None:
        try:
            await self._run_agent_phase(
                target=self.result,
                instruction=self.task.instruction,
                timeout_sec=self._agent_timeout_sec,
                user=self.task.config.agent.user,
            )
        except (AgentTimeoutError, NonZeroAgentExitCodeError) as exc:
            self._record_exception(exc)
        finally:
            await self._sync_agent_output(self.result)
"""

RUN_AGENT_PATCHED = '''    async def _run_interim_verifier(self) -> VerifierResult:
        """Run the verifier mid-trial purely to decide whether to continue."""
        mode = resolve_task_verifier_mode(self.task.config)
        user = self.task.config.verifier.user
        if mode == VerifierEnvironmentMode.SEPARATE:
            return await self._run_separate_verifier(
                key="interim",
                timeout_sec=self._verifier_timeout_sec,
                artifacts_dir=self.paths.artifacts_dir,
                user=user,
            )
        return await self._run_shared_verifier(
            timeout_sec=self._verifier_timeout_sec,
            user=user,
        )

    async def _hide_shared_verifier_tests(self) -> None:
        """Delete the verifier tests before handing control back to the agent.

        In shared-environment mode the interim verifier uploads the real tests
        into the same container the agent is about to resume in. Leaving them
        there would let the agent read the grader and write to it, which is
        cheating that we would then measure as skill.
        """
        if resolve_task_verifier_mode(self.task.config) == (
            VerifierEnvironmentMode.SEPARATE
        ):
            return
        try:
            await self.agent_environment.reset_dirs(
                remove_dirs=[self.agent_env_paths.tests_dir],
                create_dirs=[],
            )
        except Exception as exc:
            self.logger.warning(
                f"Failed to remove shared verifier tests before resuming: {exc}"
            )

    async def _run_agent(self) -> None:
        agent_cfg = self.task.config.agent
        continue_until = (
            getattr(agent_cfg, "continue_until_timeout", False)
            and self._agent_timeout_sec is not None
        )

        if not continue_until:
            try:
                await self._run_agent_phase(
                    target=self.result,
                    instruction=self.task.instruction,
                    timeout_sec=self._agent_timeout_sec,
                    user=agent_cfg.user,
                )
            except (AgentTimeoutError, NonZeroAgentExitCodeError) as exc:
                self._record_exception(exc)
            finally:
                await self._sync_agent_output(self.result)
            return

        # continue_until_timeout: loop agent -> verify -> agent until the
        # verifier fully passes or the agent budget is spent. Reaching the
        # deadline is the EXPECTED exit here, not a failure; the exception is
        # still recorded (stock behaviour) but _run() goes on to score the
        # trial normally, so a timed-out trial yields a reward, not a loss.
        deadline = time.monotonic() + self._agent_timeout_sec
        base_instruction = self.task.instruction
        instruction = base_instruction
        phase = 0
        aggregated: AgentContext | None = None
        first_started_at = None
        receipts_path = self.paths.agent_dir / "buzz" / "receipts.jsonl"
        phase_receipts: list[Path] = []
        unsettled_phases: list[int] = []

        def keep_phase_receipts(index: int) -> "Path | None":
            """Move this phase's receipts aside before the next phase truncates them.

            BuzzContainerRuntime.run() ends EVERY phase by calling bundle.write(),
            which calls accounting.write_receipts(), which opens receipts.jsonl
            with mode "w". One truncating write per phase at one path: absent this,
            the only receipts that survive a multi-phase trial are the last
            phase's. Measured on lhtb46-team before the fix --
            grammar-fuzz-coverage-hunt reported $1.76 against agent_result's $9.90
            (5.6x low), langchain-version-migration $0.03 against $2.05 (63x).
            """
            if not receipts_path.exists():
                return None
            try:
                kept = receipts_path.with_name(f"receipts.phase-{index:02d}.jsonl")
                receipts_path.replace(kept)
                phase_receipts.append(kept)
                return kept
            except OSError as exc:
                # Never let bookkeeping fail a trial that actually ran.
                self.logger.warning(f"could not keep phase {index} receipts: {exc}")
                return None

        try:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break

                try:
                    await self._run_agent_phase(
                        target=self.result,
                        instruction=instruction,
                        timeout_sec=remaining,
                        user=agent_cfg.user,
                    )
                finally:
                    # In a finally, because the phase that runs out the deadline
                    # raises AFTER the runtime's own finally has already written
                    # its receipts -- and that phase is this loop's expected
                    # ending, so the exception path is the one that most needs
                    # them kept.
                    kept_path = keep_phase_receipts(phase + 1)
                    phase_ctx = self.result.agent_result
                    if kept_path is not None and (
                        phase_ctx is None or phase_ctx.is_empty()
                    ):
                        # An empty context means this phase was cancelled before
                        # it could report -- see _context_from_receipts. Merged
                        # here rather than after the try, because the normal merge
                        # below is skipped by the exception. Guarded on is_empty()
                        # so a phase that DID report is never counted twice.
                        recovered = _context_from_receipts(kept_path)
                        if recovered is not None and (recovered.cost_usd or 0.0) > 0.0:
                            aggregated = _merge_agent_contexts(aggregated, recovered)
                        else:
                            # Receipts exist but price the phase at zero, which
                            # measured real on lhtb46-team/epidemic-inverse-
                            # control-audit: $5.45 in agent_result, $0.00 across
                            # 3 receipt rows. BuzzContainerRuntime._settle_usage
                            # runs only on its success path -- "a timeout has no
                            # usage to flush" -- so a cancelled phase's tokens
                            # were never flushed before teardown and exist
                            # NOWHERE. Merging that zero would quietly assert the
                            # phase was free; recording it instead marks the
                            # trial's cost as a floor. Do not "fix" this by
                            # summing agent_result + receipts: on this path the
                            # receipts are the zero, not the missing piece.
                            unsettled_phases.append(phase + 1)
                # _run_agent_phase stamps a FRESH TimingInfo on every call, so
                # the first phase's start has to be kept and put back at the
                # end. Without it the trial reports only the last phase's
                # duration -- a trial that worked for three hours can land in
                # the results as 1.1 seconds, quietly destroying every
                # agent-hours and $/agent-hour figure derived from it.
                if first_started_at is None:
                    first_started_at = self.result.agent_execution.started_at

                aggregated = _merge_agent_contexts(aggregated, self.result.agent_result)
                self.result.agent_result = aggregated

                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break

                interim: VerifierResult | None = None
                verifier_timed_out = False
                try:
                    interim = await self._run_interim_verifier()
                except asyncio.TimeoutError:
                    verifier_timed_out = True
                finally:
                    await self._hide_shared_verifier_tests()

                if not verifier_timed_out and _verifier_passed(interim):
                    break

                phase += 1
                feedback = (
                    "The interim verifier timed out. Keep improving the solution."
                    if verifier_timed_out
                    else _read_verifier_feedback(self.paths.verifier_dir)
                )
                instruction = _format_continue_instruction(
                    base_instruction,
                    phase=phase,
                    remaining_sec=max(0, int(remaining)),
                    feedback=feedback,
                )
                self.logger.info(
                    f"continue_until_timeout: phase {phase} starting, "
                    f"{remaining:.0f}s remaining"
                )
        except (AgentTimeoutError, NonZeroAgentExitCodeError) as exc:
            self._record_exception(exc)
        finally:
            # Restore the start here rather than per iteration: the phase that
            # runs out the deadline raises *after* stamping its own TimingInfo,
            # so a per-iteration restore leaves the last phase's start in the
            # result -- which is the common case, since spending the whole
            # budget is this loop's expected ending.
            if first_started_at is not None and self.result.agent_execution:
                self.result.agent_execution.started_at = first_started_at

            # Rebuild receipts.jsonl as every phase concatenated. Leaving only
            # the numbered per-phase files would be WORSE than the bug this
            # fixes: every analysis tool reads receipts.jsonl and sums its rows,
            # so an absent file reads as "$0 spent" rather than "data missing" --
            # the same absent-means-zero trap as docs/02 SS3.1. Concatenation is
            # the right shape because a seat's rows already sum across phases,
            # which is how every one of those tools aggregates.
            if phase_receipts:
                try:
                    with receipts_path.open("w", encoding="utf-8") as handle:
                        for part in phase_receipts:
                            handle.write(part.read_text(encoding="utf-8"))
                    receipts_path.chmod(0o600)
                except OSError as exc:
                    self.logger.warning(f"could not rebuild receipts.jsonl: {exc}")

            if aggregated is not None:
                if aggregated.metadata is None:
                    aggregated.metadata = {}
                aggregated.metadata["continue_until_timeout_phases"] = phase
                # Recorded so a later analysis can ASSERT agreement instead of
                # picking a favourite field. receipts and agent_result are built
                # from different sources -- priced usage rows scraped from the
                # container versus the context the agent itself returned -- so a
                # wide gap means one of them is wrong and the run needs looking
                # at, not averaging. A None cost means the file was unreadable.
                aggregated.metadata["receipts_phase_files"] = len(phase_receipts)
                aggregated.metadata["receipts_cost_usd"] = _sum_receipt_cost(
                    receipts_path
                )
                # Non-empty means this trial's cost is a FLOOR, not a level: those
                # phases ran and spent, and their usage was never flushed. An
                # analysis that quotes the mean cost of a cell containing these
                # should say so rather than average them in silently.
                aggregated.metadata["unsettled_phases"] = unsettled_phases
                self.result.agent_result = aggregated
            await self._sync_agent_output(self.result)
'''


def find_harbor_root(explicit: str | None) -> Path:
    if explicit:
        root = Path(explicit)
        if not (root / "trial" / "single_step.py").exists():
            raise SystemExit(f"not a harbor package dir: {root}")
        return root
    spec = importlib.util.find_spec("harbor")
    if spec is None or not spec.origin:
        raise SystemExit("harbor is not importable; pass --harbor-root")
    return Path(spec.origin).parent


def write_unlinked(path: Path, text: str) -> None:
    """Replace `path`'s contents without writing through a shared inode.

    uv populates a venv by hardlinking files out of its wheel cache, so a plain
    write_text() into site-packages edits the cached wheel too. The patch then
    survives `uv pip install --reinstall-package harbor` -- reinstall relinks
    the same, already-modified inode -- and there is no way back to a pristine
    tree short of `uv cache clean harbor`. Unlinking first gives this path a
    fresh inode and leaves the cache untouched.
    """
    path.unlink()
    path.write_text(text)


def apply_edit(path: Path, anchor: str, patched: str, *, check: bool) -> str:
    text = path.read_text()
    if patched in text:
        return "already-applied"
    if anchor not in text:
        raise SystemExit(
            f"anchor text not found in {path} -- Harbor version drifted, "
            "re-derive the patch by hand"
        )
    if check:
        return "would-apply"
    write_unlinked(path, text.replace(anchor, patched, 1))
    return "applied"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="report without writing")
    ap.add_argument("--harbor-root", default=None)
    args = ap.parse_args()

    root = find_harbor_root(args.harbor_root)
    print(f"harbor package: {root}")

    config_py = root / "models" / "task" / "config.py"
    single_py = root / "trial" / "single_step.py"
    for p in (config_py, single_py):
        if not p.exists():
            raise SystemExit(f"missing {p}")

    results = {
        "config.py:AgentConfig.continue_until_timeout": apply_edit(
            config_py, CONFIG_ANCHOR, CONFIG_PATCHED, check=args.check
        ),
        "single_step.py:imports": apply_edit(
            single_py, IMPORT_ANCHOR, IMPORT_PATCHED, check=args.check
        ),
        "single_step.py:_run_agent loop": apply_edit(
            single_py, RUN_AGENT_ANCHOR, RUN_AGENT_PATCHED, check=args.check
        ),
    }

    # Helpers are module-level, so they go in separately -- inserted ahead of
    # the class rather than inside it. The sentinel must be the *definition*,
    # not the bare name: the loop above calls _merge_agent_contexts, so a
    # substring check on the name alone reports "already applied" and silently
    # skips inserting the helpers.
    text = single_py.read_text()
    if "def _merge_agent_contexts" not in text:
        if args.check:
            results["single_step.py:module-helpers"] = "would-apply"
        else:
            class_anchor = "class SingleStepTrial(Trial):"
            if class_anchor not in text:
                raise SystemExit("could not locate SingleStepTrial class")
            write_unlinked(
                single_py,
                text.replace(
                    class_anchor, HELPERS.strip("\n") + "\n\n" + class_anchor, 1
                ),
            )
            results["single_step.py:module-helpers"] = "applied"
    else:
        results["single_step.py:module-helpers"] = "already-applied"

    for k, v in results.items():
        print(f"  {v:16s} {k}")

    if not args.check:
        import os
        import subprocess

        # Import the patched copy specifically, which may not be the harbor on
        # the default sys.path (scratch-dir dry runs, multiple venvs).
        env = dict(os.environ)
        env["PYTHONPATH"] = str(root.parent) + os.pathsep + env.get("PYTHONPATH", "")
        rc = subprocess.run(
            [
                sys.executable,
                "-c",
                "import harbor.trial.single_step as m; "
                "from harbor.models.task.config import AgentConfig; "
                "assert 'continue_until_timeout' in AgentConfig.model_fields, "
                "'config field missing'; "
                "assert hasattr(m, '_merge_agent_contexts'), 'helpers missing'; "
                "import inspect; src = inspect.getsource(m.SingleStepTrial._run_agent); "
                "assert 'continue_until_timeout_phases' in src, 'loop missing'; "
                # Asserted separately from the loop marker: the loop and the
                # per-phase receipts fix were added at different times, and a
                # re-derived patch that drops the receipts half would otherwise
                # verify green while quietly restoring the last-phase-only bug.
                "assert 'receipts.phase-' in src, 'per-phase receipts fix missing'; "
                "assert hasattr(m, '_sum_receipt_cost'), 'receipts cross-check missing'; "
                "print('verify: OK -- field, helpers, phase loop and per-phase "
                "receipts all present')",
            ],
            env=env,
        ).returncode
        if rc != 0:
            return rc
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
