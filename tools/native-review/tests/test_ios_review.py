import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

MODULE_PATH = pathlib.Path(__file__).parents[1] / "ios_review.py"
SPEC = importlib.util.spec_from_file_location("ios_review", MODULE_PATH)
ios_review = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(ios_review)


class FakeRecorder:
    def __init__(self):
        self.stderr = self
        self.finalized = False

    def readline(self):
        return "Recording started\n"

    def poll(self):
        return 0 if self.finalized else None

    def send_signal(self, _signal):
        self.finalized = True

    def wait(self, timeout):
        return 0

    def kill(self):
        self.finalized = True


class IosReviewTests(unittest.TestCase):
    def test_device_selection_prefers_latest_runtime_and_records_it(self):
        payload = {"devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-18-5": [
                {"name": "iPhone Test", "udid": "old", "isAvailable": True}
            ],
            "com.apple.CoreSimulator.SimRuntime.iOS-26-0": [
                {"name": "iPhone Test", "udid": "new", "isAvailable": True}
            ],
        }}
        completed = subprocess.CompletedProcess([], 0, json.dumps(payload), "")
        with mock.patch.object(ios_review, "run", return_value=completed):
            device = ios_review.available_device("iPhone Test")
        self.assertEqual(device["udid"], "new")
        self.assertEqual(device["runtimeIdentifier"], "com.apple.CoreSimulator.SimRuntime.iOS-26-0")

    def test_missing_device_fails_clearly(self):
        completed = subprocess.CompletedProcess([], 0, '{"devices": {}}', "")
        with mock.patch.object(ios_review, "run", return_value=completed):
            with self.assertRaisesRegex(ios_review.ReviewError, "no available"):
                ios_review.available_device("Absent")

    def test_flutter_failure_finalizes_recording_writes_receipt_and_cleans_device(self):
        recorder = FakeRecorder()
        commands = []

        def fake_run(command, **kwargs):
            commands.append(command)
            if command[:2] == ["flutter", "drive"]:
                return subprocess.CompletedProcess(command, 1, "journey failed", "diagnostic")
            if "screenshot" in command:
                pathlib.Path(command[-1]).write_bytes(b"png")
            return subprocess.CompletedProcess(command, 0, "", "")

        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(ios_review.sys, "platform", "darwin"), \
             mock.patch.object(ios_review.shutil, "which", return_value="/tool"), \
             mock.patch.object(ios_review, "provenance", return_value={"head_sha": "a" * 40, "dirty": False, "status": []}), \
             mock.patch.object(ios_review, "git", return_value="a" * 12), \
             mock.patch.object(ios_review, "available_device", return_value={"name": "iPhone Test", "udid": "device", "runtimeIdentifier": "runtime"}), \
             mock.patch.object(ios_review, "run", side_effect=fake_run), \
             mock.patch.object(ios_review, "wait_for_recording"), \
             mock.patch.object(ios_review.subprocess, "Popen", return_value=recorder):
            with self.assertRaisesRegex(ios_review.ReviewError, "exit 1"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipts = list(pathlib.Path(directory).rglob("receipt.json"))
            self.assertEqual(len(receipts), 1)
            receipt = json.loads(receipts[0].read_text())

        self.assertTrue(recorder.finalized)
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(receipt["cleanup"], {"status": "passed", "errors": []})
        self.assertEqual(receipt["artifacts"], {
            "video": "video.mp4", "log": "flutter.log", "screenshot": "final.png"
        })
        self.assertIn(["xcrun", "simctl", "shutdown", "device"], commands)
        self.assertIn(["xcrun", "simctl", "erase", "device"], commands)

    def test_recorder_timeout_is_reported_and_device_is_cleaned(self):
        recorder = FakeRecorder()
        recorder.readline = mock.Mock(return_value="")
        commands = []

        def fake_run(command, **kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 0, "", "")

        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(ios_review.sys, "platform", "darwin"), \
             mock.patch.object(ios_review.shutil, "which", return_value="/tool"), \
             mock.patch.object(ios_review, "provenance", return_value={"head_sha": "a" * 40, "dirty": False, "status": []}), \
             mock.patch.object(ios_review, "git", return_value="a" * 12), \
             mock.patch.object(ios_review, "available_device", return_value={"name": "iPhone Test", "udid": "device"}), \
             mock.patch.object(ios_review, "run", side_effect=fake_run), \
             mock.patch.object(ios_review, "wait_for_recording", side_effect=ios_review.ReviewError("timed out waiting for Simulator recording")), \
             mock.patch.object(ios_review.subprocess, "Popen", return_value=recorder):
            with self.assertRaisesRegex(ios_review.ReviewError, "timed out"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipt = json.loads(next(pathlib.Path(directory).rglob("receipt.json")).read_text())

        self.assertTrue(recorder.finalized)
        self.assertEqual(receipt["cleanup"]["status"], "passed")
        self.assertIn(["xcrun", "simctl", "erase", "device"], commands)


if __name__ == "__main__":
    unittest.main()
