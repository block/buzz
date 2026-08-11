import assert from "node:assert/strict";
import test from "node:test";

import { findNewManagedAgentFailures } from "./useManagedAgentFailureNotifications.ts";

function agent(overrides = {}) {
  return {
    pubkey: "aa".repeat(32),
    name: "Codex task",
    status: "stopped",
    lastError: "load failed",
    lastStoppedAt: "2026-08-11T09:00:00Z",
    ...overrides,
  };
}

test("reports a newly stopped managed agent exactly once", () => {
  const stopped = agent();
  const previous = new Map([[stopped.pubkey, "2026-08-11T08:00:00Z"]]);

  assert.deepEqual(findNewManagedAgentFailures(previous, [stopped]), [stopped]);
  assert.deepEqual(
    findNewManagedAgentFailures(
      new Map([[stopped.pubkey, stopped.lastStoppedAt]]),
      [stopped],
    ),
    [],
  );
});

test("does not notify for initial, running, or clean stops", () => {
  const stopped = agent();
  assert.deepEqual(findNewManagedAgentFailures(new Map(), [stopped]), []);
  assert.deepEqual(
    findNewManagedAgentFailures(new Map([[stopped.pubkey, "older"]]), [
      agent({ status: "running" }),
    ]),
    [],
  );
  assert.deepEqual(
    findNewManagedAgentFailures(new Map([[stopped.pubkey, "older"]]), [
      agent({ lastError: null }),
    ]),
    [],
  );
});
