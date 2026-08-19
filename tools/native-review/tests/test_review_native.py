import importlib.util
import json
import pathlib
import tempfile
import unittest
import urllib.error
import urllib.request
from unittest import mock

MODULE_PATH = pathlib.Path(__file__).parents[1] / "review_native.py"
SPEC = importlib.util.spec_from_file_location("review_native", MODULE_PATH)
review_native = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(review_native)


class JourneyTests(unittest.TestCase):
    def test_real_journey_validates(self):
        journey = review_native.load_journey(MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml")
        self.assertEqual(journey["flow"], "tooltip_fresh_dwell")

    def test_broken_mutation_is_schema_valid(self):
        journey = review_native.load_journey(MODULE_PATH.parent / "tests/fixtures/broken-tooltip.yaml")
        self.assertEqual(journey["steps"][0]["timeout_ms"], 50)

    def test_action_without_postcondition_is_rejected(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace("    expect:\n      exists: {role: window}\n", "", 1)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "requires name/act/expect"):
                review_native.load_journey(path)

    def test_expectation_with_multiple_conditions_is_rejected(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace(
            "    expect:\n      exists: {role: window}\n",
            "    expect:\n      exists: {role: window}\n      enabled: true\n",
            1,
        )
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "exactly one"):
                review_native.load_journey(path)

    def test_production_and_remote_targets_are_rejected(self):
        with self.assertRaisesRegex(review_native.HarnessError, "non-loopback"):
            review_native.isolation_manifest("run", "wss://buzz.block.builderlab.xyz")
        safe = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
        self.assertTrue(safe["bundle_id"].startswith("xyz.block.buzz.app.dev.native-review."))
        self.assertNotIn(safe["bundle_id"], review_native.PRODUCTION_BUNDLE_IDS)

    def test_secret_environment_is_scrubbed(self):
        sentinels = {
            "BUZZ_PRIVATE_KEY": "must-not-survive",
            "SSH_AUTH_SOCK": "/tmp/credential-agent",
            "GITHUB_TOKEN": "github-secret",
            "AWS_SECRET_ACCESS_KEY": "cloud-secret",
        }
        with mock.patch.dict(review_native.os.environ, sentinels, clear=False):
            environment = review_native.scrubbed_environment()
        for name in sentinels:
            self.assertNotIn(name, environment)

    def test_fixture_environment_is_fixed_local_and_scrubbed(self):
        sentinels = {"BUZZ_DB_HOST": "production-db", "BUZZ_DB_PASS": "production-secret",
                     "SSH_AUTH_SOCK": "/tmp/credential-agent"}
        isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
        with mock.patch.dict(review_native.os.environ, sentinels, clear=False):
            environment = review_native.fixture_environment(isolation, "a" * 64)
        self.assertEqual(environment["BUZZ_DB_HOST"], "localhost")
        self.assertEqual(environment["BUZZ_DB_PORT"], "5471")
        self.assertEqual(environment["BUZZ_DB_PASS"], "buzz_dev")
        self.assertNotIn("SSH_AUTH_SOCK", environment)
        with self.assertRaisesRegex(review_native.HarnessError, "port 3030"):
            review_native.fixture_environment(
                review_native.isolation_manifest("run", "ws://127.0.0.1:3001"), "a" * 64)

    def test_fixture_seed_failure_removes_generated_identity(self):
        generated = mock.Mock(stdout=f"Secret key: {'1' * 64}\nPublic key: {'2' * 64}\n")
        with tempfile.TemporaryDirectory() as directory:
            run_dir = pathlib.Path(directory)
            (run_dir / "manifest").mkdir()
            isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
            with mock.patch.object(review_native, "run", side_effect=[generated, RuntimeError("seed failed")]):
                with self.assertRaisesRegex(RuntimeError, "seed failed"):
                    review_native.prepare_fixture(run_dir, isolation)
            self.assertFalse((run_dir / "state/identity.key").exists())

    def test_run_scrubs_repository_controlled_commands_by_default(self):
        safe = {"PATH": "/usr/bin", "HOME": "/tmp/home"}
        with mock.patch.object(review_native, "scrubbed_environment", return_value=safe), \
             mock.patch.object(review_native.subprocess, "run", return_value=mock.Mock()) as subprocess_run:
            review_native.run(["/tmp/repository-tool"])
        self.assertEqual(subprocess_run.call_args.kwargs["env"], safe)

    def test_cleanup_uses_isolated_home_without_host_credentials(self):
        sentinels = {"BUZZ_PRIVATE_KEY": "secret", "SSH_AUTH_SOCK": "/tmp/agent",
                     "GITHUB_TOKEN": "secret", "AWS_SECRET_ACCESS_KEY": "secret"}
        with tempfile.TemporaryDirectory() as directory:
            run_dir = pathlib.Path(directory)
            isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
            with mock.patch.dict(review_native.os.environ, sentinels, clear=False), \
                 mock.patch.object(review_native, "run") as run:
                review_native.cleanup_review_state(run_dir, isolation, None)
        environment = run.call_args.kwargs["env"]
        self.assertEqual(environment["HOME"], str(run_dir / "home"))
        for name in sentinels:
            self.assertNotIn(name, environment)

    def test_cleanup_removes_identity_when_state_reset_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            run_dir = pathlib.Path(directory)
            secret_path = run_dir / "state/identity.key"
            secret_path.parent.mkdir()
            secret_path.write_text("review-secret")
            isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
            fixture = {"secret_path": str(secret_path)}
            with mock.patch.object(review_native, "run", side_effect=RuntimeError("reset failed")):
                with self.assertRaisesRegex(
                        review_native.HarnessError,
                        "desktop state reset failed: reset failed"):
                    review_native.cleanup_review_state(run_dir, isolation, fixture)
            self.assertFalse(secret_path.exists())

    def test_cleanup_aggregates_reset_and_identity_removal_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            run_dir = pathlib.Path(directory)
            secret_path = run_dir / "state/identity.key"
            isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
            fixture = {"secret_path": str(secret_path)}
            with mock.patch.object(review_native, "run", side_effect=RuntimeError("reset failed")), \
                 mock.patch.object(pathlib.Path, "unlink", side_effect=OSError("unlink failed")):
                with self.assertRaisesRegex(
                        review_native.HarnessError,
                        "desktop state reset failed: reset failed; "
                        "review identity removal failed: unlink failed"):
                    review_native.cleanup_review_state(run_dir, isolation, fixture)

    def test_repository_controlled_subprocesses_receive_scrubbed_environments(self):
        safe = {"PATH": "/usr/bin", "HOME": "/tmp/home"}
        with mock.patch.object(review_native, "scrubbed_environment", return_value=safe), \
             mock.patch.object(review_native.subprocess, "Popen") as popen:
            review_native.Driver(pathlib.Path("/tmp/driver"), 42, pathlib.Path("/tmp/snapshot"))
        self.assertEqual(popen.call_args.kwargs["env"], safe)

    def test_visible_window_waits_for_reveal(self):
        driver = mock.Mock()
        driver.request.side_effect = [
            {"ok": True, "visible": False, "detail": "not yet"},
            {"ok": True, "visible": True, "window_id": 42},
        ]
        process = mock.Mock()
        process.poll.return_value = None
        with mock.patch.object(review_native.time, "sleep"):
            status = review_native.wait_for_visible_window(driver, process, timeout_seconds=1)
        self.assertEqual(status["window_id"], 42)
        self.assertEqual(driver.request.call_count, 2)

    def test_locate_required_retries_until_target_materializes(self):
        driver = mock.Mock()
        driver.request.side_effect = [
            {"ok": True, "element": None},
            {"ok": True, "element": {"locator": {"id": "target"}}},
        ]
        with mock.patch.object(review_native.time, "sleep"):
            found = review_native.locate_required(driver, [{"id": "target"}], 1000)
        self.assertEqual(found["locator"]["id"], "target")
        self.assertEqual(driver.request.call_count, 2)

    def test_visible_window_fails_if_app_exits(self):
        driver = mock.Mock()
        process = mock.Mock()
        process.poll.return_value = 1
        with self.assertRaisesRegex(review_native.HarnessError, "exited"):
            review_native.wait_for_visible_window(driver, process, timeout_seconds=1)
        driver.request.assert_not_called()
    def test_strict_action_duration_and_metric_validation(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        mutations = [
            ("act: {type: activate}", "act: {type: activate, key: tab}", "unsupported type or fields"),
            ("duration_ms: 100}", "duration_ms: true}", "duration_ms must be"),
            ("measure: tooltip_open_latency", "measure: Invalid Metric", "lowercase metric"),
            ("measure: tooltip_open_latency", "measure: tooltip_open_latency\n    timeout_ms: true", "timeout_ms"),
        ]
        for old, new, error in mutations:
            with self.subTest(new=new), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "invalid.yaml"
                path.write_text(source.replace(old, new, 1))
                with self.assertRaisesRegex(review_native.HarnessError, error):
                    review_native.load_journey(path)

    def test_duplicate_metric_is_rejected(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace(
            "    measure: tooltip_open_latency\n  - name: leave_trigger",
            "    measure: tooltip_open_latency\n  - name: duplicate_measurement\n"
            "    act: {type: wait, duration_ms: 0}\n"
            "    expect:\n      exists: {role: tooltip, name: \"Mention someone\"}\n"
            "    expect_not_before_ms: 400\n    measure: tooltip_open_latency\n"
            "  - name: leave_trigger",
        )
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "duplicates"):
                review_native.load_journey(path)

    def test_relay_and_pubkey_validation_fail_closed(self):
        for relay in (
            "ws://127.0.0.1",
            "ws://user@127.0.0.1:3030",
            "ws://127.0.0.1:3030/path",
            "ws://127.0.0.1:3030?query",
            "ws://127.0.0.1:99999",
        ):
            with self.subTest(relay=relay), self.assertRaises(review_native.HarnessError):
                review_native.isolation_manifest("run", relay)
        isolation = review_native.isolation_manifest("run", "ws://127.0.0.1:3030")
        with self.assertRaisesRegex(review_native.HarnessError, "pubkey"):
            review_native.fixture_environment(isolation, "not-a-pubkey")

    def test_semantic_probe_requires_token_and_caps_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "semantic.json"
            server, url = review_native.semantic_probe_server(path)
            try:
                request = urllib.request.Request(url, data=b"[]", method="POST")
                with urllib.request.urlopen(request) as response:
                    self.assertEqual(response.status, 204)
                self.assertEqual(json.loads(path.read_text()), [])
                wrong = urllib.request.Request(
                    url.rsplit("/", 1)[0] + "/" + "0" * 64, data=b"[]", method="POST")
                with self.assertRaisesRegex(urllib.error.HTTPError, "404"):
                    urllib.request.urlopen(wrong)
                oversized = urllib.request.Request(
                    url, data=b"[]", method="POST",
                    headers={"Content-Length": str(review_native.MAX_PROBE_BYTES + 1)})
                with self.assertRaisesRegex(urllib.error.HTTPError, "413"):
                    urllib.request.urlopen(oversized)
            finally:
                server.shutdown()
                server.server_close()


    def test_forced_termination_is_reaped_before_cleanup(self):
        process = mock.Mock()
        process.wait.side_effect = [review_native.subprocess.TimeoutExpired("app", 15), 0]
        errors, exited = review_native.terminate_process(process)
        self.assertTrue(exited)
        self.assertEqual(errors, ["Tauri launcher required SIGKILL"])
        self.assertEqual(
            [call[0] for call in process.method_calls],
            ["terminate", "wait", "kill", "wait"],
        )

    def test_unconfirmed_exit_preserves_isolated_state(self):
        process = mock.Mock()
        process.wait.side_effect = [
            review_native.subprocess.TimeoutExpired("app", 15),
            review_native.subprocess.TimeoutExpired("app", 5),
        ]
        with mock.patch.object(review_native, "cleanup_review_state") as cleanup:
            errors = review_native.cleanup_process_and_state(
                process, pathlib.Path("/tmp/run"), {}, None)
        cleanup.assert_not_called()
        self.assertEqual(
            errors,
            ["Tauri launcher did not exit after SIGKILL; isolated state was preserved"],
        )
        self.assertEqual(
            [call[0] for call in process.method_calls],
            ["terminate", "wait", "kill", "wait"],
        )

    def test_cleanup_cannot_be_disabled(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace("terminate_app: true", "terminate_app: false")
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "cleanup is mandatory"):
                review_native.load_journey(path)

    def test_lower_bound_requires_measurement(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace("    measure: tooltip_open_latency\n", "", 1)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "requires measure"):
                review_native.load_journey(path)

    def test_measurement_requires_causal_start_and_lower_bound(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        mutations = [
            ("    measure_start: tooltip_open_latency\n", "", "earlier measure_start"),
            ("    expect_not_before_ms: 400\n", "", "requires expect_not_before_ms"),
        ]
        for old, new, error in mutations:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "invalid.yaml"
                path.write_text(source.replace(old, new, 1))
                with self.assertRaisesRegex(review_native.HarnessError, error):
                    review_native.load_journey(path)

    def test_lower_bound_rejects_early_and_late_postconditions(self):
        driver = mock.Mock()
        expectation = {"exists": {"id": "tooltip"}}
        driver.request.return_value = {"ok": True, "element": {"locator": {"id": "tooltip"}}}
        with mock.patch.object(review_native.time, "monotonic_ns", return_value=100_000_000):
            with self.assertRaisesRegex(review_native.HarnessError, "before 400ms"):
                review_native.wait_expectation_not_before(
                    driver, expectation, start_ns=0, lower_bound_ms=400, timeout_ms=250)

        driver.request.return_value = {"ok": True, "element": None}
        with mock.patch.object(review_native.time, "monotonic_ns", return_value=650_000_000):
            with self.assertRaisesRegex(review_native.HarnessError, "not met"):
                review_native.wait_expectation_not_before(
                    driver, expectation, start_ns=0, lower_bound_ms=400, timeout_ms=250)

    def test_lower_bound_records_first_valid_observation(self):
        driver = mock.Mock()
        expectation = {"exists": {"id": "tooltip"}}
        driver.request.return_value = {"ok": True, "element": {"locator": {"id": "tooltip"}}}
        with mock.patch.object(review_native.time, "monotonic_ns", return_value=500_000_000):
            observed = review_native.wait_expectation_not_before(
                driver, expectation, start_ns=0, lower_bound_ms=400, timeout_ms=250)
        self.assertEqual(observed, 500_000_000)

    def test_measurement_receipt_shape_is_durable(self):
        duration_ms = 12.5
        receipt = {"measurements": {"tooltip_open_latency": {"value": duration_ms, "unit": "ms"}}}
        self.assertEqual(json.loads(json.dumps(receipt))["measurements"]["tooltip_open_latency"]["value"], duration_ms)


if __name__ == "__main__":
    unittest.main()
