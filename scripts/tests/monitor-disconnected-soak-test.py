#!/usr/bin/env python3
"""Functional tests for the bounded disconnected soak monitor."""

from __future__ import annotations

import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[2]
MONITOR = ROOT / "scripts" / "monitor-disconnected-soak.py"


class SoakMonitorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.audit = self.root / "audit.db"
        self.cloud_log = self.root / "app.log"
        self.cloud_log.write_text("prior online history\n", encoding="utf-8")
        self.monitored = self.root / "state"
        self.monitored.mkdir()
        self.report = self.root / "report.json"
        self.probe = self.root / "probe.py"
        self.probe.write_text(
            """#!/usr/bin/env python3
import json
import os
from pathlib import Path

if os.environ.get("PROBE_APPEND_CLOUD"):
    with Path(os.environ["PROBE_APPEND_CLOUD"]).open("a", encoding="utf-8") as output:
        output.write("provider=cloud outbound attempt\\n")
if os.environ.get("PROBE_GROW_PATH"):
    Path(os.environ["PROBE_GROW_PATH"]).write_bytes(b"x" * 2048)
ready = os.environ.get("PROBE_READY", "1") == "1"
print(json.dumps({
    "ready": ready,
    "components_ready": ready,
    "network": {"disconnected_observed": ready},
    "components": {"model": {"instance_id": "gemma4-26b-official"}}
}))
""",
            encoding="utf-8",
        )
        self.probe.chmod(0o755)
        with sqlite3.connect(self.audit) as database:
            database.executescript(
                """
                CREATE TABLE command_brief_spool (
                    run_id TEXT NOT NULL,
                    event_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    publish_state TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE command_brief_schedule_claims (
                    run_id TEXT NOT NULL,
                    state TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                """
            )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_monitor(self, env: dict[str, str] | None = None, resume: bool = False):
        command = [
            "python3",
            str(MONITOR),
            "--probe-program",
            str(self.probe),
            "--audit-db",
            str(self.audit),
            "--cloud-log",
            str(self.cloud_log),
            "--monitor-dir",
            str(self.monitored),
            "--report",
            str(self.report),
            "--duration-seconds",
            "0",
            "--interval-seconds",
            "0.01",
            "--grace-samples",
            "0",
            "--max-active-seconds",
            "10",
            "--max-growth-bytes",
            "1024",
            "--max-growth-percent",
            "100",
        ]
        if resume:
            command.append("--resume")
        process_env = os.environ.copy()
        process_env.update(env or {})
        return subprocess.run(command, env=process_env, text=True, capture_output=True)

    def test_healthy_sample_and_resume_pass(self):
        first = self.run_monitor()
        self.assertEqual(first.returncode, 0, first.stderr)
        first_report = json.loads(self.report.read_text(encoding="utf-8"))
        second = self.run_monitor(resume=True)
        self.assertEqual(second.returncode, 0, second.stderr)
        second_report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertEqual(second_report["started_at"], first_report["started_at"])
        self.assertGreater(second_report["sample_count"], first_report["sample_count"])

    def test_new_cloud_attempt_fails(self):
        result = self.run_monitor({"PROBE_APPEND_CLOUD": str(self.cloud_log)})
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("cloud_attempt_observed", report["failures"])

    def test_stuck_new_schedule_claim_fails(self):
        now = int(time.time())
        with sqlite3.connect(self.audit) as database:
            database.execute(
                "INSERT INTO command_brief_schedule_claims VALUES(?,?,?)",
                ("run-stuck", "started", now - 30),
            )
        result = self.run_monitor(env={"SOAK_START_EPOCH_OVERRIDE": str(now - 60)})
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("stuck_command_brief_run", report["failures"])

    def test_duplicate_new_publication_fails(self):
        now = int(time.time())
        with sqlite3.connect(self.audit) as database:
            database.executemany(
                "INSERT INTO command_brief_spool VALUES(?,?,?,?,?)",
                [
                    ("run-1", "event-1", "complete", "published", now),
                    ("run-1", "event-2", "complete", "published", now),
                ],
            )
        result = self.run_monitor(env={"SOAK_START_EPOCH_OVERRIDE": str(now)})
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("duplicate_brief_publication", report["failures"])

    def test_component_loss_fails_after_grace(self):
        result = self.run_monitor({"PROBE_READY": "0"})
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("component_readiness_lost", report["failures"])

    def test_excessive_disk_growth_fails(self):
        result = self.run_monitor(
            {"PROBE_GROW_PATH": str(self.monitored / "growth.bin")}
        )
        self.assertNotEqual(result.returncode, 0)
        report = json.loads(self.report.read_text(encoding="utf-8"))
        self.assertIn("disk_growth_exceeded", report["failures"])


if __name__ == "__main__":
    unittest.main()
