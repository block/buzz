import importlib.util
import json
import pathlib
import re
import tempfile
import unittest
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

    def test_broader_journeys_and_mutations_validate(self):
        expected = {
            "desktop/composer-keyboard.yaml": "composer_keyboard",
            "desktop/search-shortcut-dismissal.yaml": "search_shortcut_dismissal",
            "tests/fixtures/broken-text.yaml": "broken_text",
            "tests/fixtures/broken-shortcut.yaml": "broken_shortcut",
            "tests/fixtures/broken-scroll.yaml": "broken_scroll",
        }
        for relative_path, flow in expected.items():
            with self.subTest(relative_path=relative_path):
                journey = review_native.load_journey(MODULE_PATH.parent / relative_path)
                self.assertEqual(journey["flow"], flow)

    def test_duplicate_measurement_is_rejected(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        source = source.replace("  - name: leave_trigger\n", "  - name: duplicate_measure\n    act: {type: wait, duration_ms: 1}\n    expect: {exists: {role: window}}\n    measure: tooltip_open_latency\n  - name: leave_trigger\n")
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "duplicate measurement"):
                review_native.load_journey(path)

    def test_type_text_requires_text(self):
        source = (MODULE_PATH.parent / "desktop/composer-keyboard.yaml").read_text()
        source = re.sub(r'\{type: type_text, text: "[^"]*"\}', "{type: type_text}", source, count=1)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "type_text requires text"):
                review_native.load_journey(path)

    def test_scroll_requires_integer_delta(self):
        source = (MODULE_PATH.parent / "desktop/composer-keyboard.yaml").read_text()
        source = re.sub(r"\{type: scroll, delta_y: -?240\}", "{type: scroll, delta_y: nope}", source, count=1)
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid.yaml"
            path.write_text(source)
            with self.assertRaisesRegex(review_native.HarnessError, "scroll requires integer delta_y"):
                review_native.load_journey(path)


    def test_boolean_and_nonfinite_journey_numbers_are_rejected(self):
        source = (MODULE_PATH.parent / "desktop/tooltip-fresh-dwell.yaml").read_text()
        composer = (MODULE_PATH.parent / "desktop/composer-keyboard.yaml").read_text()
        cases = [
            (source, "timeout_ms: 15000", "timeout_ms: true", "timeout_ms"),
            (source, "duration_ms: 800", "duration_ms: true", "duration_ms"),
            (composer, "scroll_y_less_than: 1", "scroll_y_less_than: .nan", "finite number"),
        ]
        for source_text, old, new, error in cases:
            with self.subTest(new=new), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "invalid.yaml"
                path.write_text(source_text.replace(old, new, 1))
                with self.assertRaisesRegex(review_native.HarnessError, error):
                    review_native.load_journey(path)

    def test_scroll_requires_locator_and_bounded_delta(self):
        source = (MODULE_PATH.parent / "desktop/composer-keyboard.yaml").read_text()
        without_locator = source.replace("    locate:\n      - {id: message-input-scroll}\n", "", 1)
        oversized = source.replace("delta_y: 240", "delta_y: 10001", 1)
        for value in (without_locator, oversized):
            with tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / "invalid.yaml"
                path.write_text(value)
                with self.assertRaises(review_native.HarnessError):
                    review_native.load_journey(path)

    def test_relay_url_rejects_ambiguous_components(self):
        for relay in (
            "ws://localhost:3030/path", "ws://user@localhost:3030",
            "ws://localhost:3030?x=1", "ws://localhost:3030#x",
            "ws://localhost", "ws://localhost:0", "ws://localhost:99999",
        ):
            with self.subTest(relay=relay), self.assertRaises(review_native.HarnessError):
                review_native.isolation_manifest("run", relay)

    def test_value_expectation_uses_selected_element(self):
        driver = mock.Mock()
        driver.request.return_value = {"ok": True, "element": {"value": "draft"}}
        self.assertTrue(review_native.expectation_holds(driver, {"value": "draft"}))
        self.assertFalse(review_native.expectation_holds(driver, {"value": "wrong"}))

    def test_scroll_expectation_uses_selected_element(self):
        driver = mock.Mock()
        driver.request.return_value = {"ok": True, "element": {"scrollY": 240}}
        self.assertTrue(review_native.expectation_holds(driver, {"scroll_y_greater_than": 0}))
        self.assertFalse(review_native.expectation_holds(driver, {"scroll_y_greater_than": 240}))
        self.assertTrue(review_native.expectation_holds(driver, {"scroll_y_less_than": 241}))
        self.assertFalse(review_native.expectation_holds(driver, {"scroll_y_less_than": 240}))

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


class PerformanceTests(unittest.TestCase):
    MACHINE = {"host_id_sha256": "f" * 64, "system": "Darwin", "release": "test", "machine": "arm64", "cpu": "test"}

    def receipt(self, path, artifact, timing, cpu=10, memory=100, flow="tooltip_fresh_dwell"):
        payload = {
            "schema_version": 1,
            "run_id": path.stem, "flow": flow, "status": "passed",
            "cleanup": {"status": "passed"},
            "provenance": {"dirty": False, "dirty_state_sha256": None, "status": [],
                           "head_sha": ("a" if artifact == "base" else "b") * 40,
                           "artifact_sha256": ("c" if artifact == "base" else "d") * 64},
            "measurements": {"tooltip_open_latency": {"value": timing, "unit": "ms", "step": "tooltip"}},
            "performance": {"machine": self.MACHINE, "process": {
                "cpu_percent_median": cpu, "resident_mb_peak": memory,
            }},
        }
        path.write_text(json.dumps(payload))
        return path

    def budget(self, path, regression=20):
        path.write_text(f"""schema_version: 1
flow: tooltip_fresh_dwell
minimum_samples: 3
metrics:
  tooltip_open_latency:
    max: 1000
    max_regression_percent: {regression}
  process.resident_mb_peak:
    max: 500
    max_regression_percent: 20
""")
        return path

    def test_comparison_uses_median_and_passes_with_noise(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", value) for i, value in enumerate((100, 101, 900))]
            candidate = [self.receipt(root / f"c{i}.json", "head", value) for i, value in enumerate((110, 111, 5))]
            result = review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["baseline"]["metrics"]["tooltip_open_latency"]["median"], 101)

    def test_comparison_fails_when_one_sample_exceeds_absolute_maximum(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", value)
                         for i, value in enumerate((100, 100, 10000))]
            with self.assertRaisesRegex(review_native.HarnessError, "maximum 10000.000 exceeds absolute maximum 1000"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))

    def test_comparison_fails_on_relative_regression(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 130) for i in range(3)]
            with self.assertRaisesRegex(review_native.HarnessError, "regression"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))

    def test_comparison_rejects_too_few_samples(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(2)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(2)]
            with self.assertRaisesRegex(review_native.HarnessError, "at least 3"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))


    def test_comparison_requires_explicit_clean_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            for dirty in ("missing", 0, None):
                path = self.receipt(root / f"r-{dirty}.json", "base", 100)
                payload = json.loads(path.read_text())
                if dirty == "missing":
                    del payload["provenance"]["dirty"]
                else:
                    payload["provenance"]["dirty"] = dirty
                path.write_text(json.dumps(payload))
                with self.subTest(dirty=dirty), self.assertRaisesRegex(
                        review_native.HarnessError, "clean provenance"):
                    review_native.load_receipts([path], "baseline")

    def test_comparison_rejects_nonfinite_bool_overlap_and_same_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            payload = json.loads(candidate[0].read_text())
            payload["measurements"]["tooltip_open_latency"]["value"] = float("nan")
            candidate[0].write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.HarnessError, "finite numeric"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))

            candidate = [self.receipt(root / f"d{i}.json", "base", 100) for i in range(3)]
            with self.assertRaisesRegex(review_native.HarnessError, "same source revision"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))
            with self.assertRaisesRegex(review_native.HarnessError, "overlap"):
                review_native.compare_performance(baseline, baseline, self.budget(root / "budget.yaml"))

    def test_comparison_rejects_bool_and_nonfinite_limits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            for value in ("true", ".nan", ".inf"):
                budget = self.budget(root / f"budget-{value}.yaml")
                budget.write_text(budget.read_text().replace("max: 1000", f"max: {value}"))
                with self.assertRaisesRegex(review_native.HarnessError, "invalid limits"):
                    review_native.compare_performance(baseline, candidate, budget)

    def test_comparison_rejects_incompatible_machine(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            payload = json.loads(candidate[0].read_text())
            payload["performance"]["machine"]["machine"] = "x86_64"
            candidate[0].write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.HarnessError, "incompatible machines"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))

    def test_comparison_rejects_different_same_model_host(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            for path in candidate:
                payload = json.loads(path.read_text())
                payload["performance"]["machine"]["host_id_sha256"] = "e" * 64
                path.write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.HarnessError, "incompatible machines"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))

    def test_benchmark_reuses_first_run_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact = root / "buzz-desktop"
            artifact.write_text("immutable")
            calls = []

            def fake_run_journey(_path, _relay, output, *, app_binary, isolation_id):
                calls.append((app_binary, isolation_id))
                run_dir = output / f"run-{len(calls)}"
                run_dir.mkdir(parents=True)
                (run_dir / "receipt.json").write_text(json.dumps({
                    "provenance": {
                        "artifact_path": str(artifact),
                        "artifact_sha256": review_native.sha256(artifact),
                    }
                }))
                return run_dir

            with mock.patch.object(review_native, "run_journey", side_effect=fake_run_journey):
                receipts = review_native.benchmark(pathlib.Path("journey.yaml"), "ws://localhost:3030", root, 3)
            self.assertEqual(len(receipts), 3)
            self.assertIsNone(calls[0][0])
            prepared = root / ".cohorts" / calls[0][1] / "buzz-desktop"
            self.assertEqual([call[0] for call in calls[1:]], [prepared, prepared])
            self.assertEqual(prepared.read_text(), "immutable")
            self.assertEqual(len({call[1] for call in calls}), 1)

    def test_comparison_rejects_mixed_revisions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            payload = json.loads(candidate[0].read_text())
            payload["provenance"]["head_sha"] = "e" * 40
            candidate[0].write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.HarnessError, "mixes source revisions"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))


if __name__ == "__main__":
    unittest.main()
