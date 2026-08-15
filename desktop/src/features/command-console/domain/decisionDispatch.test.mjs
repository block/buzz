import assert from "node:assert/strict";
import test from "node:test";

import {
  buildCommandDirectionMessage,
  parseCommandDirectionStatus,
} from "./decisionDispatch.ts";

test("builds one direct Chief of Staff instruction with no source padding", () => {
  const message = buildCommandDirectionMessage({
    directionId: "run-1:action-1",
    decision: "Choose the readiness response.",
    direction: "Complete the readiness review today.",
  });
  assert.match(message, /CO DIRECTION run-1:action-1/);
  assert.match(message, /Complete the readiness review today\./);
  assert.match(message, /connected systems/i);
  assert.match(message, /IN PROGRESS.*COMPLETE.*BLOCKED.*FAILED/s);
  assert.doesNotMatch(message, /sourceIds|ledger|citation/i);
});

test("maps only the matching Chief status line", () => {
  assert.deepEqual(
    parseCommandDirectionStatus(
      "CO DIRECTION run-1:action-1 — COMPLETE\nCalendar and task list updated.",
      "run-1:action-1",
    ),
    {
      status: "completed",
      statusText: "Calendar and task list updated.",
    },
  );
  assert.deepEqual(
    parseCommandDirectionStatus(
      "CO DIRECTION run-1:action-1 - IN PROGRESS",
      "run-1:action-1",
    ),
    { status: "in_progress", statusText: "Chief of Staff is working." },
  );
  assert.deepEqual(
    parseCommandDirectionStatus(
      "**CO DIRECTION run-1:action-1 — BLOCKED**\nAwaiting an updated programme.",
      "run-1:action-1",
    ),
    { status: "blocked", statusText: "Awaiting an updated programme." },
  );
  assert.equal(
    parseCommandDirectionStatus(
      "CO DIRECTION another-action — FAILED",
      "run-1:action-1",
    ),
    null,
  );
});
