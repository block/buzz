#!/usr/bin/env python3
"""Exercise the actual user-deletion CLI against an explicit disposable database."""

import argparse
import json
import os
import subprocess
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    database_url = os.environ.get("BUZZ_USER_DELETION_TEST_DATABASE_URL")
    if not database_url:
        parser.error("set BUZZ_USER_DELETION_TEST_DATABASE_URL to an isolated, migrated database")
    environment = dict(os.environ, DATABASE_URL=database_url)
    binary = str(args.binary.resolve())

    def command(*parts, expected=0):
        result = subprocess.run(
            [binary, "user-deletions", *parts],
            env=environment,
            capture_output=True,
            text=True,
            timeout=90,
            check=False,
        )
        if result.returncode != expected:
            raise AssertionError(
                f"{parts}: expected exit {expected}, got {result.returncode}\n"
                f"{result.stdout[-8000:]}\n{result.stderr[-8000:]}"
            )
        if expected != 0:
            return None
        return json.loads(result.stdout)

    def prepare():
        target = "demo:cli-" + uuid.uuid4().hex
        request = command("submit", "--target", target, "--requested-by", "operator-a")
        request_id = request["id"]
        repeated = command("submit", "--target", target, "--requested-by", "operator-a")
        assert repeated["id"] == request_id
        command("run", request_id, expected=5)
        inventory = command("inventory", request_id)
        assert len(inventory["inventory_manifest"]["communities"]) == 3
        assert all(c["relationship"] == "participant"
                   for c in inventory["inventory_manifest"]["communities"])
        command("approve", request_id, "--approved-by", "operator-a", expected=5)
        command("approve", request_id, "--approved-by", "operator-b")
        return request_id

    def finished(request_id):
        result = command("inspect", request_id)
        assert result["request"]["status"] == "demo_completed"
        assert result["request"]["lease_owner"] is None
        history = result["stage_history"]
        assert len(history) == 15
        assert all(not s["is_current"] and s["status"] == "completed" for s in history)
        assert all(s["metadata_json"]["simulated"] for s in history)
        assert [s["stage"] for s in history[:4]] == [
            "gate_user", "gate_community", "gate_community", "gate_community"
        ]
        for start, community in [(4, 1), (7, 2), (10, 3)]:
            assert all(s["target_community_id"] == str(uuid.UUID(int=community))
                       for s in history[start:start + 3])
        return result

    stopped_id = prepare()
    command("run", stopped_id, "--stop-after", "7")
    stopped = command("inspect", stopped_id)
    assert stopped["request"]["lease_owner"] is None
    assert len(stopped["stage_history"]) == 8
    assert stopped["stage_history"][-1]["status"] == "pending"
    command("run", stopped_id)
    completed = finished(stopped_id)
    assert completed["stage_history"][:7] == stopped["stage_history"][:7]
    print(f"PASS CLI stop/resume: {stopped_id}", flush=True)

    failed_id = prepare()
    command("run", failed_id, "--fail-at", "9", expected=1)
    failed = command("inspect", failed_id)
    assert len(failed["stage_history"]) == 9
    assert failed["stage_history"][-1]["status"] == "failed"
    assert failed["request"]["lease_owner"] is None
    retry_at = datetime.fromisoformat(failed["request"]["next_attempt_at"].replace("Z", "+00:00"))
    delay = max(0, (retry_at - datetime.now(timezone.utc)).total_seconds()) + 0.2
    assert delay < 35, "unexpected retry delay"
    time.sleep(delay)
    command("run", failed_id)
    recovered = finished(failed_id)
    assert recovered["stage_history"][:8] == failed["stage_history"][:8]
    assert recovered["stage_history"][8]["id"] == failed["stage_history"][8]["id"]
    assert recovered["request"]["lease_generation"] == 2
    print(f"PASS CLI failure in B/retry: {failed_id}", flush=True)

    interrupted_id = prepare()
    process = subprocess.Popen(
        [binary, "user-deletions", "run", interrupted_id, "--step-delay-ms", "30000"],
        env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        # Cold database connection setup has its own retry budget. Once the
        # stage starts, require renewal within two heartbeat intervals.
        deadline = time.monotonic() + 45
        initial = None
        while time.monotonic() < deadline:
            if process.poll() is not None:
                stdout, stderr = process.communicate(timeout=5)
                raise AssertionError(
                    f"worker exited before heartbeat: {process.returncode}\n"
                    f"{stdout[-8000:]}\n{stderr[-8000:]}"
                )
            current = command("inspect", interrupted_id)
            if current["stage_history"][0]["status"] == "running":
                if initial is None:
                    initial = current
                    deadline = time.monotonic() + 20
                elif current["request"]["lease_until"] != initial["request"]["lease_until"]:
                    break
            time.sleep(0.25)
        else:
            raise AssertionError("worker did not renew its lease while a dummy action ran")
        command("run", interrupted_id, expected=5)
        process.terminate()
        stdout, stderr = process.communicate(timeout=15)
        assert process.returncode == 0, (stdout[-8000:], stderr[-8000:])
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=10)
    interrupted = command("inspect", interrupted_id)
    assert interrupted["request"]["lease_owner"] is None
    assert interrupted["stage_history"][0]["is_current"]
    command("run", interrupted_id)
    resumed = finished(interrupted_id)
    assert resumed["request"]["lease_generation"] == 2
    assert resumed["stage_history"][0]["id"] == interrupted["stage_history"][0]["id"]
    assert resumed["stage_history"][0]["started_at"] == interrupted["stage_history"][0]["started_at"]
    print(f"PASS CLI heartbeat/SIGTERM/resume: {interrupted_id}", flush=True)
    print("PASS: all actions were simulated; no owned-community path was exercised.")


if __name__ == "__main__":
    main()
