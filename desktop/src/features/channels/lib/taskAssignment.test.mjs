import assert from "node:assert/strict";
import { test } from "node:test";
import { assignmentCanvas, taskAssignee } from "./taskAssignment.ts";

const canvas = `---
buzz_schema: channel-backed-task/v1
# keep this note
custom_field: hello
assignee: null
---
Unchanged body.\n`;
const pubkey = "a".repeat(64);

test("assignment preserves unrelated frontmatter, comments and body", () => {
  const result = assignmentCanvas(canvas, pubkey, "operation", "pending");
  assert.match(result, /# keep this note\ncustom_field: hello/);
  assert.ok(result.endsWith("---\nUnchanged body.\n"));
  assert.deepEqual(taskAssignee(result), {
    pubkey,
    notification: { id: "operation", status: "pending" },
  });
});

test("retry retains operation and refuses another assignee", () => {
  const pending = assignmentCanvas(canvas, pubkey, "operation", "pending");
  assert.equal(
    taskAssignee(assignmentCanvas(pending, pubkey, "operation", "sent"))
      .notification.status,
    "sent",
  );
  assert.throws(
    () => assignmentCanvas(pending, "b".repeat(64), "new", "pending"),
    /already assigned/,
  );
  assert.throws(
    () => assignmentCanvas(canvas, "bad", "new", "pending"),
    /Invalid agent/,
  );
});
