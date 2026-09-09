import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("assign_task", Path(__file__).with_name("assign-task-channel.py"))
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class AssignmentTests(unittest.TestCase):
    def setUp(self):
        self.helper = module.load_helper()
        self.content = "before"
        self.calls = []
        self.fail_key = False
        self.fail_send = False
        self.uncertain_send = False
        self.fail_membership = False
        self.fail_final_save = False
        self.request = dict(channel_id="12345678-1234-1234-1234-123456789abc", assignee_pubkey="a" * 64, operation_id="87654321-1234-1234-1234-123456789abc", expected_canvas="before", canvas_content="pending", completed_canvas="sent")
        self.helper.current_identity_pubkey = lambda args, env=None: "b" * 64 if env else "c" * 64
        self.helper.monitor_private_key = self.key
        self.helper.run_buzz = self.run_buzz
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def key(self):
        if self.fail_key:
            raise RuntimeError("dedicated Monitor key unavailable")
        return "monitor-key"

    def run_buzz(self, args, command, *, content=None, env=None):
        self.calls.append((command, env))
        if command[:2] == ["canvas", "get"]:
            return self.content
        if command[:2] == ["channels", "members"]:
            return []
        if command[:2] == ["canvas", "set"]:
            if content == "sent" and self.fail_final_save:
                raise RuntimeError("save failed")
            self.content = content
        if command[:2] == ["channels", "add-member"] and self.fail_membership:
            raise RuntimeError("membership denied")
        if command[:2] == ["messages", "send"]:
            if self.uncertain_send:
                raise RuntimeError("network timeout")
            if self.fail_send:
                return {"accepted": False}
            self.assertEqual(env["BUZZ_PRIVATE_KEY"], "monitor-key")
            self.assertNotIn("BUZZ_AUTH_TAG", env)
            self.assertEqual(command[-2:], ["--mention", "a" * 64])
        return {"accepted": True, "event_id": "receipt"}

    def assign(self):
        return module.assign(self.request, self.helper, Path(self.temp.name))

    def test_save_before_wake_and_retry_skips_delivered_message(self):
        self.assertTrue(self.assign()["notified"])
        self.assertTrue(self.assign()["notified"])
        self.assertEqual(sum(command[:2] == ["messages", "send"] for command, _ in self.calls), 1)

    def test_missing_monitor_preserves_assignment_without_human_fallback(self):
        self.fail_key = True
        self.assertFalse(self.assign()["notified"])
        self.assertEqual(self.content, "pending")
        self.assertFalse(any(command[0] == "messages" for command, _ in self.calls))
        self.fail_key = False
        self.assertTrue(self.assign()["notified"])

    def test_rejected_notification_can_retry(self):
        self.fail_send = True
        self.assertFalse(self.assign()["notified"])
        self.assertEqual(self.content, "pending")
        self.fail_send = False
        self.assertTrue(self.assign()["notified"])

    def test_changed_canvas_is_not_overwritten(self):
        self.content = "someone else's edit"
        with self.assertRaisesRegex(RuntimeError, "Canvas changed"):
            self.assign()
        self.assertEqual(self.content, "someone else's edit")

    def test_membership_failure_does_not_wake(self):
        self.fail_membership = True
        self.assertFalse(self.assign()["notified"])
        self.assertEqual(self.content, "pending")
        self.assertFalse(any(command[0] == "messages" for command, _ in self.calls))

    def test_uncertain_notification_does_not_blindly_resend(self):
        self.uncertain_send = True
        self.assertFalse(self.assign()["notified"])
        self.uncertain_send = False
        self.assertIn("uncertain", self.assign()["error"])
        self.assertEqual(sum(command[0] == "messages" for command, _ in self.calls), 1)

    def test_delivered_notification_with_failed_final_save_retries_only_save(self):
        self.fail_final_save = True
        self.assertFalse(self.assign()["notified"])
        self.fail_final_save = False
        self.assertTrue(self.assign()["notified"])
        self.assertEqual(sum(command[0] == "messages" for command, _ in self.calls), 1)


if __name__ == "__main__":
    unittest.main()
