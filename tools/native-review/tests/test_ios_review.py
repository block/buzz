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
    DEVICE_PAYLOAD = {
        "devicetypes": [{"name": "iPhone Test", "identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-Test"}],
        "runtimes": [
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-9-3", "version": "9.3",
             "platform": "iOS", "isAvailable": True, "supportedDeviceTypes": [
                 {"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-Test"},
             ]},
            {"identifier": "com.apple.CoreSimulator.SimRuntime.iOS-26-0", "version": "26.0",
             "platform": "iOS", "isAvailable": True, "supportedDeviceTypes": [
                 {"identifier": "com.apple.CoreSimulator.SimDeviceType.iPhone-Test"},
             ]},
        ],
    }

    def test_device_creation_uses_unique_owned_name_and_latest_runtime(self):
        responses = [
            subprocess.CompletedProcess([], 0, json.dumps(self.DEVICE_PAYLOAD), ""),
            subprocess.CompletedProcess([], 0, "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE\n", ""),
        ]
        with mock.patch.object(ios_review, "run", side_effect=responses) as run:
            device = ios_review.create_review_device("iPhone Test", "ios-run-123")
        create = run.call_args_list[1].args[0]
        self.assertEqual(create[:3], ["xcrun", "simctl", "create"])
        self.assertEqual(create[3], "Buzz Native Review ios-run-123")
        self.assertEqual(create[-1], "com.apple.CoreSimulator.SimRuntime.iOS-26-0")
        self.assertTrue(device["owned"])
        self.assertEqual(device["udid"], "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE")

    def test_missing_device_type_fails_without_creating_or_erasing_any_device(self):
        completed = subprocess.CompletedProcess([], 0, json.dumps({"devicetypes": [], "runtimes": []}), "")
        with mock.patch.object(ios_review, "run", return_value=completed) as run:
            with self.assertRaisesRegex(ios_review.ReviewError, "no iOS Simulator device type"):
                ios_review.create_review_device("Absent", "run")
        commands = [call.args[0] for call in run.call_args_list]
        self.assertFalse(any("erase" in command or "delete" in command for command in commands))

    def test_flutter_environment_scrubs_host_credentials(self):
        sentinels = {"BUZZ_PRIVATE_KEY": "secret", "SSH_AUTH_SOCK": "/tmp/agent", "GITHUB_TOKEN": "secret"}
        with mock.patch.dict(ios_review.os.environ, sentinels, clear=False):
            environment = ios_review.flutter_environment()
        for name in sentinels:
            self.assertNotIn(name, environment)

    def test_run_scrubs_host_credentials_by_default(self):
        sentinels = {"BUZZ_PRIVATE_KEY": "secret", "SSH_AUTH_SOCK": "/tmp/agent", "GITHUB_TOKEN": "secret"}
        with mock.patch.dict(ios_review.os.environ, sentinels, clear=False), \
             mock.patch.object(ios_review.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            ios_review.run(["xcrun", "simctl", "list"])
        environment = run.call_args.kwargs["env"]
        for name in sentinels:
            self.assertNotIn(name, environment)

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
             mock.patch.object(ios_review, "create_review_device", return_value={
                 "name": "Buzz Native Review owned-run", "udid": "owned-device",
                 "runtimeIdentifier": "runtime", "deviceType": "iPhone Test", "owned": True,
             }), \
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
        self.assertEqual(receipt["device"], {
            "name": "Buzz Native Review owned-run", "device_type": "iPhone Test",
            "udid": "owned-device", "runtime": "runtime", "owned": True,
        })
        self.assertIn(["xcrun", "simctl", "shutdown", "owned-device"], commands)
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)
        self.assertFalse(any("erase" in command for command in commands))
        self.assertTrue(all("pre-existing-device" not in command for command in commands))

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
             mock.patch.object(ios_review, "create_review_device", return_value={
                 "name": "Buzz Native Review timeout-run", "udid": "owned-device",
                 "runtimeIdentifier": "runtime", "deviceType": "iPhone Test", "owned": True,
             }), \
             mock.patch.object(ios_review, "run", side_effect=fake_run), \
             mock.patch.object(ios_review, "wait_for_recording", side_effect=ios_review.ReviewError("timed out waiting for Simulator recording")), \
             mock.patch.object(ios_review.subprocess, "Popen", return_value=recorder):
            with self.assertRaisesRegex(ios_review.ReviewError, "timed out"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipt = json.loads(next(pathlib.Path(directory).rglob("receipt.json")).read_text())

        self.assertTrue(recorder.finalized)
        self.assertEqual(receipt["cleanup"]["status"], "passed")
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)
        self.assertFalse(any("erase" in command for command in commands))


if __name__ == "__main__":
    unittest.main()
