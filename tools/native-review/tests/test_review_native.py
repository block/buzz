import importlib.util
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
        old = review_native.os.environ.get("BUZZ_PRIVATE_KEY")
        review_native.os.environ["BUZZ_PRIVATE_KEY"] = "must-not-survive"
        try:
            self.assertNotIn("BUZZ_PRIVATE_KEY", review_native.scrubbed_environment())
        finally:
            if old is None:
                review_native.os.environ.pop("BUZZ_PRIVATE_KEY", None)
            else:
                review_native.os.environ["BUZZ_PRIVATE_KEY"] = old

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


if __name__ == "__main__":
    unittest.main()
