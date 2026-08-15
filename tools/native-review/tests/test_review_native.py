import importlib.util
import json
import pathlib
import re
import tempfile
import subprocess
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
    MACHINE = {"system": "Darwin", "release": "test", "machine": "arm64", "cpu": "test"}

    def receipt(self, path, artifact, timing, cpu=10, memory=100, flow="tooltip_fresh_dwell"):
        payload = {
            "run_id": path.stem, "flow": flow, "status": "passed",
            "cleanup": {"status": "passed"},
            "provenance": {"dirty": False, "head_sha": artifact + "-sha", "artifact_sha256": artifact},
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
            receipts = [self.receipt(root / f"r{i}.json", "same", 100) for i in range(2)]
            with self.assertRaisesRegex(review_native.HarnessError, "at least 3"):
                review_native.compare_performance(receipts, receipts, self.budget(root / "budget.yaml"))

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

    def test_comparison_rejects_mixed_revisions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            baseline = [self.receipt(root / f"b{i}.json", "base", 100) for i in range(3)]
            candidate = [self.receipt(root / f"c{i}.json", "head", 100) for i in range(3)]
            payload = json.loads(candidate[0].read_text())
            payload["provenance"]["head_sha"] = "other"
            candidate[0].write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.HarnessError, "mixes source revisions"):
                review_native.compare_performance(baseline, candidate, self.budget(root / "budget.yaml"))


class PublishReviewTests(unittest.TestCase):
    def inputs(self, root):
        video = root / "video-share.mp4"
        video.write_bytes(b"video")
        receipt = root / "receipt.json"
        receipt.write_text(json.dumps({
            "flow": "journey", "status": "failed",
            "provenance": {"dirty": False, "head_sha": "a" * 40},
            "cleanup": {"status": "passed"},
            "artifacts": {"share_video": video.name},
        }))
        summary = root / "summary.md"
        summary.write_text("Found a regression.")
        return receipt, summary

    @mock.patch.object(review_native.review_publish.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}")
    @mock.patch.object(review_native.review_publish, "_video_duration", return_value=12.5)
    @mock.patch.object(review_native.review_publish, "_run")
    def test_publishes_video_root_then_timecoded_highlights(self, run, _duration, _which):
        run.side_effect = [
            subprocess.CompletedProcess([], 0, json.dumps({"accepted": True, "event_id": "b" * 64}), ""),
            subprocess.CompletedProcess([], 0, json.dumps({"accepted": True, "event_id": "c" * 64}), ""),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            receipt, summary = self.inputs(root)
            highlights = root / "highlights.json"
            highlights.write_text(json.dumps([{"seconds": 3.25, "text": "The broken state appears."}]))
            result = review_native.publish_review(receipt, summary, "channel", "thread", highlights, ["d" * 64])
        first, second = [call.args[0] for call in run.call_args_list]
        self.assertIn("--file", first)
        self.assertIn("--mention", first)
        self.assertEqual(second[-1], "[00:03.250] The broken state appears.")
        self.assertEqual(second[second.index("--reply-to") + 1], "b" * 64)
        self.assertEqual(result["highlight_event_ids"], ["c" * 64])

    @mock.patch.object(review_native.review_publish.shutil, "which", return_value="/usr/bin/tool")
    @mock.patch.object(review_native.review_publish, "_video_duration", return_value=10.0)
    def test_rejects_highlight_outside_video_before_publish(self, _duration, _which):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            receipt, summary = self.inputs(root)
            highlights = root / "highlights.json"
            highlights.write_text(json.dumps([{"seconds": 11, "text": "impossible"}]))
            with self.assertRaisesRegex(review_native.PublishError, "within the video"):
                review_native.publish_review(receipt, summary, "channel", "thread", highlights)

    @mock.patch.object(review_native.review_publish.shutil, "which", return_value="/usr/bin/tool")
    def test_rejects_dirty_receipt_before_upload(self, _which):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            receipt, summary = self.inputs(root)
            payload = json.loads(receipt.read_text())
            payload["provenance"]["dirty"] = True
            receipt.write_text(json.dumps(payload))
            with self.assertRaisesRegex(review_native.PublishError, "clean source"):
                review_native.publish_review(receipt, summary, "channel", "thread")


if __name__ == "__main__":
    unittest.main()
