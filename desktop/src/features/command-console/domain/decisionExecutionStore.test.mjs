import assert from "node:assert/strict";
import test from "node:test";

import {
  createDecisionExecution,
  markSilentExecutionStalled,
  parseDecisionExecutions,
  updateDecisionExecution,
} from "./decisionExecutionStore.ts";

const NOW = Date.parse("2026-08-15T01:00:00Z");

test("creates and immutably advances a compact execution record", () => {
  const queued = createDecisionExecution({
    key: "run-1:action-1",
    runId: "run-1",
    actionId: "action-1",
    direction: "Proceed with COA A.",
    directionSource: "coa_a",
    now: NOW,
  });
  assert.equal(queued.status, "queued");

  const underway = updateDecisionExecution(queued, {
    status: "in_progress",
    agentPubkey: "a".repeat(64),
    channelId: "00000000-0000-4000-8000-000000000001",
    now: NOW + 1_000,
  });
  assert.equal(underway.status, "in_progress");
  assert.equal(queued.status, "queued");
  assert.equal(Object.isFrozen(underway), true);
});

test("marks only silent non-terminal work stalled after five minutes", () => {
  const queued = createDecisionExecution({
    key: "run-1:action-1",
    runId: "run-1",
    actionId: "action-1",
    direction: "Proceed.",
    directionSource: "user",
    now: NOW,
  });
  assert.equal(
    markSilentExecutionStalled(queued, NOW + 299_999).status,
    "queued",
  );
  assert.equal(
    markSilentExecutionStalled(queued, NOW + 300_000).status,
    "stalled",
  );

  const complete = updateDecisionExecution(queued, {
    status: "completed",
    now: NOW + 10_000,
  });
  assert.equal(
    markSilentExecutionStalled(complete, NOW + 900_000).status,
    "completed",
  );
});

test("parses restart state and drops malformed or duplicate records", () => {
  const valid = createDecisionExecution({
    key: "run-1:action-1",
    runId: "run-1",
    actionId: "action-1",
    direction: "Proceed.",
    directionSource: "coa_a",
    now: NOW,
  });
  const parsed = parseDecisionExecutions(
    JSON.stringify({ version: 1, executions: [valid, valid, { bad: true }] }),
  );
  assert.deepEqual(parsed, [valid]);
  assert.equal(parseDecisionExecutions("not-json").length, 0);
});
