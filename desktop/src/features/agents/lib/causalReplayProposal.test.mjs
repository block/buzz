import assert from "node:assert/strict";
import test from "node:test";

import { buildApprovedReplayProposal } from "./causalReplayProposal.ts";

const candidate = {
  schema: "causal-experiment/v1",
  experimentId: "live:agent:session-1",
  recordedAt: "2026-08-05T00:00:00Z",
  task: { description: "Write the file", sourceMessageId: "message-1" },
  execution: { sessionId: "session-1", turnId: "turn-1", replayOf: null },
  failureFingerprint: "unclassified-live-candidate/v1",
  context: {
    codeVersion: "code-1",
    policyVersion: "policy-1",
    modelVersion: "model-1",
    toolVersion: "tool-1",
    environmentVersion: "env-1",
  },
  hypothesis: { cause: "unclassified", evidenceIds: [] },
  intervention: { remedyId: "unclassified", changedVariable: "unclassified" },
  result: { outcome: "untested", evidenceIds: ["receipt-1"] },
  coverage: { acp_observer: "observed", os_sandbox: "missing" },
  relations: { supports: [], contradicts: [], invalidates: [] },
};

test("builds one owner-approved controlled replay without inventing a result", () => {
  const proposal = buildApprovedReplayProposal(
    candidate,
    {
      failureFingerprint: "host-write-bypass/v1",
      cause: "The host tool bypassed the governed path",
      changedVariable: "execution path: host tool → ACP workspace tool",
      successCriteria:
        "Permission is requested before the write and the task succeeds",
    },
    { experimentId: "proposal-1", recordedAt: "2026-08-05T01:00:00Z" },
  );

  assert.equal(proposal.execution.replayOf, candidate.experimentId);
  assert.equal(proposal.intervention.approvedAt, "2026-08-05T01:00:00Z");
  assert.equal(proposal.result.outcome, "untested");
  assert.deepEqual(proposal.result.evidenceIds, []);
  assert.deepEqual(proposal.hypothesis.evidenceIds, ["receipt-1"]);
});

test("requires every causal and evaluation field before approval", () => {
  assert.throws(
    () =>
      buildApprovedReplayProposal(
        candidate,
        {
          failureFingerprint: "host-write-bypass/v1",
          cause: "A cause",
          changedVariable: "one variable",
          successCriteria: " ",
        },
        { experimentId: "proposal-1", recordedAt: "2026-08-05T01:00:00Z" },
      ),
    /Success criteria is required/,
  );
});

test("refuses to replay an already evaluated candidate", () => {
  assert.throws(
    () =>
      buildApprovedReplayProposal(
        { ...candidate, result: { outcome: "validated", evidenceIds: [] } },
        {
          failureFingerprint: "failure/v1",
          cause: "A cause",
          changedVariable: "one variable",
          successCriteria: "A measured result",
        },
        { experimentId: "proposal-1", recordedAt: "2026-08-05T01:00:00Z" },
      ),
    /Only an untested candidate/,
  );
});
