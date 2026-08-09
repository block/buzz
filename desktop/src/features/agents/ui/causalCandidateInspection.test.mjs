import assert from "node:assert/strict";
import test from "node:test";

import { inspectCausalCandidate } from "./causalCandidateInspection.ts";

function entry(sessionId, overrides = {}) {
  return {
    sequence: 1,
    previousHash: "0".repeat(64),
    hash: "1".repeat(64),
    experiment: {
      schema: "causal-experiment/v1",
      experimentId: `live:agent:${sessionId}`,
      recordedAt: "2026-08-05T00:00:00Z",
      task: { description: "Write the file", sourceMessageId: null },
      execution: { sessionId, turnId: "turn-1", replayOf: null },
      failureFingerprint: "unclassified-live-candidate/v1",
      context: {},
      hypothesis: { cause: "unclassified", evidenceIds: [] },
      intervention: {
        remedyId: "unclassified",
        changedVariable: "unclassified",
      },
      result: { outcome: "untested", evidenceIds: ["receipt-1"] },
      coverage: {
        acp_observer: "observed",
        acp_permission_gate: "missing",
        host_workspace: "missing",
        os_sandbox: "missing",
      },
      relations: { supports: [], contradicts: [], invalidates: [] },
      ...overrides,
    },
  };
}

test("keeps captured facts, claims, and missing coverage separate", () => {
  const inspection = inspectCausalCandidate([entry("session-1")], "session-1");
  assert.equal(inspection?.status, "untested");
  assert.deepEqual(inspection?.inferredClaims, []);
  assert.match(
    inspection?.capturedFacts[1] ?? "",
    /activity evidence was captured/,
  );
  assert.deepEqual(inspection?.missingCoverage, [
    "Agent Client Protocol permission decisions",
    "Host workspace effects",
    "Operating-system sandbox effects",
  ]);
  assert.match(inspection?.nextGate ?? "", /independent evaluator/);
});

test("selects the latest ledger entry for the active session", () => {
  const inspection = inspectCausalCandidate(
    [
      entry("other"),
      entry("session-1", { result: { outcome: "validated", evidenceIds: [] } }),
    ],
    "session-1",
  );
  assert.equal(inspection?.status, "validated");
});

test("does not show a candidate from another session", () => {
  assert.equal(inspectCausalCandidate([entry("other")], "session-1"), null);
});
