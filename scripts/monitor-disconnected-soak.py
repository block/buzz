#!/usr/bin/env python3
"""Monitor an installed Command Adviser during a bounded disconnected soak."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
import time
from typing import Sequence


SCHEMA_VERSION = 1
CLOUD_PATTERN = re.compile(
    r"(?:provider\s*=\s*cloud|cloud_attempt|api\.openai\.com|litellm|"
    r"configured_model[^\n]*(?:chatgpt|gpt-))",
    re.IGNORECASE,
)


def iso_time(epoch: float | None = None) -> str:
    value = datetime.fromtimestamp(epoch or time.time(), tz=timezone.utc)
    return value.isoformat().replace("+00:00", "Z")


def directory_size(path: Path) -> int:
    total = 0
    if not path.exists():
        return 0
    for root, directories, files in os.walk(path, followlinks=False):
        root_path = Path(root)
        directories[:] = [
            name for name in directories if not (root_path / name).is_symlink()
        ]
        for name in files:
            candidate = root_path / name
            if candidate.is_file() and not candidate.is_symlink():
                total += candidate.stat().st_size
    return total


def atomic_write(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=str(path.parent)
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary_name, path)
    finally:
        try:
            os.unlink(temporary_name)
        except FileNotFoundError:
            pass


def audit_snapshot(database_path: Path, start_epoch: int, now_epoch: int) -> dict:
    with sqlite3.connect(f"file:{database_path}?mode=ro", uri=True) as database:
        active = database.execute(
            """
            SELECT run_id, updated_at
            FROM command_brief_schedule_claims
            WHERE state = 'started' AND updated_at >= ?
            ORDER BY run_id
            """,
            (start_epoch,),
        ).fetchall()
        duplicates = database.execute(
            """
            SELECT run_id, COUNT(DISTINCT event_id)
            FROM command_brief_spool
            WHERE publish_state = 'published' AND created_at >= ?
            GROUP BY run_id
            HAVING COUNT(DISTINCT event_id) > 1
            ORDER BY run_id
            """,
            (start_epoch,),
        ).fetchall()
        publications = database.execute(
            """
            SELECT run_id, event_id
            FROM command_brief_spool
            WHERE publish_state = 'published' AND created_at >= ?
            ORDER BY run_id, event_id
            """,
            (start_epoch,),
        ).fetchall()
    return {
        "active_runs": [
            {
                "run_id_hash": hashlib.sha256(run_id.encode("utf-8")).hexdigest(),
                "active_age_seconds": max(0, now_epoch - int(updated_at)),
            }
            for run_id, updated_at in active
        ],
        "duplicate_run_hashes": [
            hashlib.sha256(run_id.encode("utf-8")).hexdigest()
            for run_id, _count in duplicates
        ],
        "publications": [
            {
                "run_id_hash": hashlib.sha256(run_id.encode("utf-8")).hexdigest(),
                "publication_id": event_id,
            }
            for run_id, event_id in publications
        ],
    }


def read_new_cloud_attempts(path: Path, offset: int) -> tuple[int, int]:
    if not path.exists():
        return 0, offset
    size = path.stat().st_size
    if size < offset:
        offset = 0
    with path.open("r", encoding="utf-8", errors="replace") as source:
        source.seek(offset)
        appended = source.read()
        next_offset = source.tell()
    return len(CLOUD_PATTERN.findall(appended)), next_offset


def run_probe(program: Path, arguments: Sequence[str]) -> dict:
    completed = subprocess.run(
        [str(program), *arguments],
        text=True,
        capture_output=True,
        timeout=900,
        check=False,
    )
    if completed.returncode not in (0, 1):
        return {"ready": False, "probe_error": "probe_execution_failed"}
    try:
        result = json.loads(completed.stdout.strip().splitlines()[-1])
    except (IndexError, json.JSONDecodeError):
        return {"ready": False, "probe_error": "probe_report_invalid"}
    if not isinstance(result, dict):
        return {"ready": False, "probe_error": "probe_report_invalid"}
    return result


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe-program", required=True, type=Path)
    parser.add_argument("--probe-arg", action="append", default=[])
    parser.add_argument("--audit-db", required=True, type=Path)
    parser.add_argument("--cloud-log", action="append", required=True, type=Path)
    parser.add_argument("--monitor-dir", action="append", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--duration-seconds", type=float, default=8 * 60 * 60)
    parser.add_argument("--interval-seconds", type=float, default=60)
    parser.add_argument("--grace-samples", type=int, default=2)
    parser.add_argument("--max-active-seconds", type=int, default=45 * 60)
    parser.add_argument("--max-growth-bytes", type=int, default=1024 * 1024 * 1024)
    parser.add_argument("--max-growth-percent", type=float, default=10.0)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(argv)
    if (
        args.duration_seconds < 0
        or args.interval_seconds <= 0
        or args.grace_samples < 0
        or args.max_active_seconds <= 0
        or args.max_growth_bytes < 0
        or args.max_growth_percent < 0
    ):
        parser.error("monitor bounds must be non-negative and intervals positive")
    return args


def initial_state(args: argparse.Namespace) -> dict:
    start_epoch = int(os.environ.get("SOAK_START_EPOCH_OVERRIDE", str(int(time.time()))))
    baseline_sizes = {
        str(path.expanduser().resolve()): directory_size(path.expanduser())
        for path in args.monitor_dir
    }
    cloud_offsets = {
        str(path.expanduser().resolve()): path.stat().st_size if path.exists() else 0
        for path in args.cloud_log
    }
    return {
        "schema_version": SCHEMA_VERSION,
        "started_at": iso_time(float(start_epoch)),
        "start_epoch": start_epoch,
        "sample_count": 0,
        "consecutive_unready_samples": 0,
        "cloud_log_offsets": cloud_offsets,
        "cloud_attempt_count": 0,
        "baseline_directory_bytes": baseline_sizes,
        "current_directory_bytes": dict(baseline_sizes),
        "failures": [],
        "samples": [],
        "result": "running",
    }


def load_state(args: argparse.Namespace) -> dict:
    if not args.resume:
        return initial_state(args)
    try:
        state = json.loads(args.report.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"unable to resume soak report: {error}") from error
    if state.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unable to resume unsupported soak report")
    state["result"] = "running"
    state.pop("completed_at", None)
    return state


def add_failure(state: dict, failure: str) -> None:
    if failure not in state["failures"]:
        state["failures"].append(failure)


def sample_once(args: argparse.Namespace, state: dict) -> None:
    now = int(time.time())
    probe = run_probe(args.probe_program, args.probe_arg)
    if probe.get("ready") is True:
        state["consecutive_unready_samples"] = 0
    else:
        state["consecutive_unready_samples"] += 1
        if state["consecutive_unready_samples"] > args.grace_samples:
            add_failure(state, "component_readiness_lost")

    try:
        audit = audit_snapshot(args.audit_db, int(state["start_epoch"]), now)
    except (OSError, sqlite3.Error):
        audit = {"active_runs": [], "duplicate_run_hashes": [], "publications": []}
        add_failure(state, "command_brief_audit_unavailable")
    if any(
        item["active_age_seconds"] > args.max_active_seconds
        for item in audit["active_runs"]
    ):
        add_failure(state, "stuck_command_brief_run")
    if audit["duplicate_run_hashes"]:
        add_failure(state, "duplicate_brief_publication")

    cloud_attempts = 0
    for path in args.cloud_log:
        key = str(path.expanduser().resolve())
        new_attempts, next_offset = read_new_cloud_attempts(
            path, int(state["cloud_log_offsets"].get(key, 0))
        )
        state["cloud_log_offsets"][key] = next_offset
        cloud_attempts += new_attempts
    state["cloud_attempt_count"] += cloud_attempts
    if state["cloud_attempt_count"]:
        add_failure(state, "cloud_attempt_observed")

    current_sizes = {
        str(path.expanduser().resolve()): directory_size(path.expanduser())
        for path in args.monitor_dir
    }
    state["current_directory_bytes"] = current_sizes
    for path, baseline in state["baseline_directory_bytes"].items():
        growth = current_sizes.get(path, 0) - int(baseline)
        percentage = (growth / max(1, int(baseline))) * 100
        if growth > args.max_growth_bytes or (
            int(baseline) > 0 and percentage > args.max_growth_percent
        ):
            add_failure(state, "disk_growth_exceeded")

    model = probe.get("components", {}).get("model", {})
    sample = {
        "sampled_at": iso_time(float(now)),
        "ready": probe.get("ready") is True,
        "model_instance_id": model.get("instance_id"),
        "active_run_count": len(audit["active_runs"]),
        "publication_count": len(audit["publications"]),
        "cloud_attempt_count": state["cloud_attempt_count"],
        "directory_bytes": sum(current_sizes.values()),
    }
    state["samples"].append(sample)
    state["sample_count"] += 1


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv if argv is not None else sys.argv[1:])
    try:
        state = load_state(args)
    except ValueError as error:
        print(f"disconnected soak failed: {error}", file=sys.stderr)
        return 2
    deadline = time.monotonic() + args.duration_seconds
    while True:
        sample_once(args, state)
        state["result"] = "failed" if state["failures"] else "running"
        atomic_write(args.report, state)
        if state["failures"] or time.monotonic() >= deadline:
            break
        time.sleep(args.interval_seconds)
    state["completed_at"] = iso_time()
    state["result"] = "failed" if state["failures"] else "pass"
    atomic_write(args.report, state)
    print(
        json.dumps(
            {
                "result": state["result"],
                "sample_count": state["sample_count"],
                "failures": state["failures"],
            }
        )
    )
    return 0 if state["result"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
