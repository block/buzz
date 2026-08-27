import importlib.util
import json
import pathlib
import subprocess
import tempfile
import threading
import unittest
import urllib.request
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


class FakeFlutterProcess:
    def __init__(self, returncode=0):
        self.returncode = returncode
        self.running = True
        self.terminated = False

    def poll(self):
        return None if self.running else self.returncode

    def wait(self, timeout=None):
        self.running = False
        return self.returncode

    def terminate(self):
        self.terminated = True
        self.running = False

    def kill(self):
        self.terminated = True
        self.running = False


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

    def test_flutter_environment_carries_private_proceed_url(self):
        environment = ios_review.flutter_environment("http://127.0.0.1:1/proceed/token")
        self.assertEqual(environment["BUZZ_NATIVE_REVIEW_PROCEED_URL"], "http://127.0.0.1:1/proceed/token")
        self.assertEqual(environment["SIMCTL_CHILD_BUZZ_NATIVE_REVIEW_PROCEED_URL"], environment["BUZZ_NATIVE_REVIEW_PROCEED_URL"])

    def test_run_scrubs_host_credentials_by_default(self):
        sentinels = {"BUZZ_PRIVATE_KEY": "secret", "SSH_AUTH_SOCK": "/tmp/agent", "GITHUB_TOKEN": "secret"}
        with mock.patch.dict(ios_review.os.environ, sentinels, clear=False), \
             mock.patch.object(ios_review.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run:
            ios_review.run(["xcrun", "simctl", "list"])
        environment = run.call_args.kwargs["env"]
        for name in sentinels:
            self.assertNotIn(name, environment)

    def test_proceed_gate_waits_for_explicit_recorder_release(self):
        server, url = ios_review.proceed_server()
        result = []

        def request_proceed():
            with urllib.request.urlopen(url, timeout=2) as response:
                result.append(response.status)

        request = threading.Thread(target=request_proceed)
        request.start()
        try:
            self.assertFalse(server.review_ready.is_set())
            request.join(timeout=0.05)
            self.assertTrue(request.is_alive())
            ios_review.release_journey(server)
            request.join(timeout=1)
            self.assertFalse(request.is_alive())
            self.assertEqual(result, [204])
        finally:
            ios_review.release_journey(server)
            server.shutdown()
            server.server_close()
            request.join(timeout=1)

    def test_state_evidence_requires_every_reviewed_transition_in_order(self):
        with tempfile.TemporaryDirectory() as directory:
            log = pathlib.Path(directory) / "flutter.log"
            log.write_text("\n".join(
                f"{ios_review.STATE_MARKER_PREFIX}{state}" for state in
                ["initial-hidden", "revealed", "edited", "final-hidden"]
            ))
            self.assertEqual(
                [step["name"] for step in ios_review.evidence_steps(log)],
                ["initial-hidden", "revealed", "edited", "final-hidden"],
            )
            log.write_text(f"{ios_review.STATE_MARKER_PREFIX}final-hidden\n")
            with self.assertRaisesRegex(ios_review.ReviewError, "evidence incomplete"):
                ios_review.evidence_steps(log)

    def test_second_reap_timeout_is_collected_without_aborting_teardown(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.side_effect = [
            subprocess.TimeoutExpired("flutter", 10),
            subprocess.TimeoutExpired("flutter", 5),
        ]
        errors = ios_review.terminate_child(process, "Flutter")
        self.assertEqual(errors, ["Flutter required SIGKILL", "Flutter did not exit after SIGKILL"])
        process.kill.assert_called_once()

    def test_all_simctl_commands_and_flutter_waits_are_bounded(self):
        source = MODULE_PATH.read_text()
        self.assertIn(
            '["xcrun", "simctl", "list", "devicetypes", "runtimes", "-j"],\n'
            '        timeout=SIMCTL_TIMEOUT_SECONDS',
            source,
        )
        self.assertIn(
            'device_type["identifier"], runtime["identifier"]],\n'
            '        timeout=SIMCTL_TIMEOUT_SECONDS',
            source,
        )
        self.assertIn(
            '"bootstatus", udid, "-b"], capture=False, '
            'timeout=BOOT_TIMEOUT_SECONDS',
            source,
        )
        self.assertIn(
            "flutter_process.wait(timeout=FLUTTER_TIMEOUT_SECONDS)", source
        )
        self.assertIn(
            '"screenshot", str(screenshot)], timeout=SIMCTL_TIMEOUT_SECONDS',
            source,
        )

    def test_recording_readiness_requires_journey_marker(self):
        process = FakeFlutterProcess()
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(ios_review.time, "monotonic", side_effect=[0, 2]), \
             mock.patch.object(ios_review.time, "sleep"):
            with self.assertRaisesRegex(ios_review.ReviewError, "timed out waiting"):
                ios_review.wait_for_recording_ready(
                    process, pathlib.Path(directory) / "flutter.log", timeout_seconds=1
                )

    def test_recording_readiness_accepts_rendered_journey_marker(self):
        process = FakeFlutterProcess()
        with tempfile.TemporaryDirectory() as directory:
            log = pathlib.Path(directory) / "flutter.log"
            log.write_text(f"build output\n{ios_review.RECORDING_READY_MARKER}\n")
            ios_review.wait_for_recording_ready(process, log, timeout_seconds=0.1)

    def test_flutter_failure_finalizes_recording_writes_receipt_and_cleans_device(self):
        recorder = FakeRecorder()
        flutter = FakeFlutterProcess(1)
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
             mock.patch.object(ios_review, "wait_for_recording_ready"), \
             mock.patch.object(ios_review, "wait_for_recording"), \
             mock.patch.object(ios_review, "evidence_steps", return_value=[{"name": "state", "status": "passed"}]), \
             mock.patch.object(ios_review, "finalize_recording") as finalize_recording, \
             mock.patch.object(ios_review.subprocess, "Popen", side_effect=[flutter, recorder]):
            with self.assertRaisesRegex(ios_review.ReviewError, "exit 1"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipts = list(pathlib.Path(directory).rglob("receipt.json"))
            self.assertEqual(len(receipts), 1)
            receipt = json.loads(receipts[0].read_text())

        finalize_recording.assert_called_once()
        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(receipt["cleanup"], {"status": "passed", "errors": []})
        self.assertEqual(receipt["artifacts"], {
            "video": "video.mp4", "log": "flutter.log"
        })
        self.assertEqual(receipt["isolation"]["device"], {
            "name": "Buzz Native Review owned-run", "device_type": "iPhone Test",
            "udid": "owned-device", "runtime": "runtime", "owned": True,
        })
        self.assertEqual(receipt["flow"], "ios_pairing")
        self.assertIn("started_at", receipt)
        self.assertIn("finished_at", receipt)
        self.assertEqual(receipt["steps"], [])
        self.assertEqual(receipt["measurements"], {})
        self.assertIn("machine", receipt["performance"])
        schema = json.loads((MODULE_PATH.parent / "schemas/receipt.schema.json").read_text())
        self.assertTrue(set(schema["required"]) <= set(receipt))
        self.assertFalse(set(receipt) - set(schema["properties"]))
        self.assertIn(["xcrun", "simctl", "launch", "owned-device", ios_review.IOS_BUNDLE_ID], commands)
        self.assertIn(["xcrun", "simctl", "shutdown", "owned-device"], commands)
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)
        self.assertFalse(any("erase" in command for command in commands))
        self.assertTrue(all("pre-existing-device" not in command for command in commands))

    def test_recorder_failure_after_start_fails_successful_journey_and_cleans_device(self):
        recorder = FakeRecorder()
        flutter = FakeFlutterProcess()
        recorder.finalized = True
        recorder.poll = mock.Mock(return_value=9)
        recorder.wait = mock.Mock(return_value=9)
        recorder.read = mock.Mock(return_value="encoder failed")
        commands = []

        def fake_run(command, **kwargs):
            commands.append(command)
            if command[:2] == ["flutter", "drive"]:
                return subprocess.CompletedProcess(command, 0, "journey passed", "")
            if "screenshot" in command:
                pathlib.Path(command[-1]).write_bytes(b"png")
            return subprocess.CompletedProcess(command, 0, "", "")

        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(ios_review.sys, "platform", "darwin"), \
             mock.patch.object(ios_review.shutil, "which", return_value="/tool"), \
             mock.patch.object(ios_review, "provenance", return_value={"head_sha": "a" * 40, "dirty": False, "status": []}), \
             mock.patch.object(ios_review, "git", return_value="a" * 12), \
             mock.patch.object(ios_review, "create_review_device", return_value={
                 "name": "Buzz Native Review failed-recorder", "udid": "owned-device",
                 "runtimeIdentifier": "runtime", "deviceType": "iPhone Test", "owned": True,
             }), \
             mock.patch.object(ios_review, "run", side_effect=fake_run), \
             mock.patch.object(ios_review, "wait_for_recording_ready"), \
             mock.patch.object(ios_review, "wait_for_recording"), \
             mock.patch.object(ios_review, "evidence_steps", return_value=[{"name": "state", "status": "passed"}]), \
             mock.patch.object(ios_review.subprocess, "Popen", side_effect=[flutter, recorder]):
            with self.assertRaisesRegex(ios_review.ReviewError, "recorder failed with exit 9"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipt = json.loads(next(pathlib.Path(directory).rglob("receipt.json")).read_text())

        self.assertEqual(receipt["status"], "failed")
        self.assertIn("recorder failed with exit 9", receipt["failure"])
        self.assertEqual(receipt["cleanup"]["status"], "failed")
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)

    def test_video_validation_timeout_fails_journey_and_still_cleans_device(self):
        recorder = FakeRecorder()
        flutter = FakeFlutterProcess()
        commands = []

        def fake_run(command, **kwargs):
            commands.append(command)
            if command[:2] == ["flutter", "drive"]:
                return subprocess.CompletedProcess(command, 0, "journey passed", "")
            if "screenshot" in command:
                pathlib.Path(command[-1]).write_bytes(b"png")
            if command[0] == "/usr/bin/avconvert":
                raise subprocess.TimeoutExpired(command, kwargs["timeout"])
            return subprocess.CompletedProcess(command, 0, "", "")

        def fake_wait_for_recording(_recorder):
            run_dir = next((pathlib.Path(directory) / ("a" * 12) / "ios_pairing").iterdir())
            (run_dir / "video.mp4").write_bytes(b"video")

        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(ios_review.sys, "platform", "darwin"), \
             mock.patch.object(ios_review.shutil, "which", return_value="/tool"), \
             mock.patch.object(ios_review, "provenance", return_value={"head_sha": "a" * 40, "dirty": False, "status": []}), \
             mock.patch.object(ios_review, "git", return_value="a" * 12), \
             mock.patch.object(ios_review, "create_review_device", return_value={
                 "name": "Buzz Native Review timeout", "udid": "owned-device",
                 "runtimeIdentifier": "runtime", "deviceType": "iPhone Test", "owned": True,
             }), \
             mock.patch.object(ios_review, "run", side_effect=fake_run), \
             mock.patch.object(ios_review, "wait_for_recording_ready"), \
             mock.patch.object(ios_review, "wait_for_recording", side_effect=fake_wait_for_recording), \
             mock.patch.object(ios_review, "evidence_steps", return_value=[{"name": "state", "status": "passed"}]), \
             mock.patch.object(ios_review.subprocess, "Popen", side_effect=[flutter, recorder]):
            with self.assertRaisesRegex(ios_review.ReviewError, "timed out validating simulator video"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipt = json.loads(next(pathlib.Path(directory).rglob("receipt.json")).read_text())

        self.assertEqual(receipt["status"], "failed")
        self.assertEqual(receipt["failure"], "timed out validating simulator video")
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)

    def test_recorder_timeout_is_reported_and_device_is_cleaned(self):
        recorder = FakeRecorder()
        flutter = FakeFlutterProcess()
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
             mock.patch.object(ios_review, "wait_for_recording_ready"), \
             mock.patch.object(ios_review, "wait_for_recording", side_effect=ios_review.ReviewError("timed out waiting for Simulator recording")), \
             mock.patch.object(ios_review, "finalize_recording", side_effect=lambda process, _video: process.send_signal(0)), \
             mock.patch.object(ios_review.subprocess, "Popen", side_effect=[flutter, recorder]):
            with self.assertRaisesRegex(ios_review.ReviewError, "timed out"):
                ios_review.run_review(ios_review.DEFAULT_TEST, "iPhone Test", pathlib.Path(directory))
            receipt = json.loads(next(pathlib.Path(directory).rglob("receipt.json")).read_text())

        self.assertTrue(recorder.finalized)
        self.assertEqual(receipt["cleanup"]["status"], "passed")
        self.assertIn(["xcrun", "simctl", "delete", "owned-device"], commands)
        self.assertFalse(any("erase" in command for command in commands))


if __name__ == "__main__":
    unittest.main()
