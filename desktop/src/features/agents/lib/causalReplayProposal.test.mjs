import assert from "node:assert/strict";
import test from "node:test";

import {
  buildApprovedReplayProposal,
  buildIndependentEvaluation,
  buildReplayDispatchMessage,
  buildReplayDispatchReceipt,
  proposalIdFromReplayTask,
} from "./causalReplayProposal.ts";

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

test("dispatches an approved replay with a durable correlation marker", () => {
  const proposal = buildApprovedReplayProposal(
    candidate,
    {
      failureFingerprint: "failure/v1",
      cause: "A cause",
      changedVariable: "one variable",
      successCriteria: "A measured result",
    },
    { experimentId: "proposal-1", recordedAt: "2026-08-05T01:00:00Z" },
  );
  const message = buildReplayDispatchMessage(proposal);
  assert.equal(proposalIdFromReplayTask(message), "proposal-1");
  assert.match(message, /disposable workspace/);
  assert.match(message, /Change exactly one variable: one variable/);

  const receipt = buildReplayDispatchReceipt(
    proposal,
    "event-1",
    "2026-08-05T01:01:00Z",
  );
  assert.equal(receipt.task.sourceMessageId, "event-1");
  assert.equal(receipt.execution.replayOf, proposal.experimentId);
  assert.deepEqual(receipt.result.evidenceIds, ["message:event-1"]);
});

test("requires cited evidence before sealing an independent verdict", () => {
  const replay = {
    ...candidate,
    experimentId: "replay-1",
    execution: { ...candidate.execution, replayOf: "proposal-1" },
  };
  assert.throws(
    () =>
      buildIndependentEvaluation(
        replay,
        { outcome: "validated", evidenceIds: " ", rationale: "It worked" },
        { experimentId: "evaluation-1", recordedAt: "2026-08-05T02:00:00Z" },
      ),
    /At least one evidence ID/,
  );
  const evaluation = buildIndependentEvaluation(
    replay,
    {
      outcome: "rejected",
      evidenceIds: "observer:1\nreceipt:2",
      rationale: "The success criterion was not met.",
    },
    { experimentId: "evaluation-1", recordedAt: "2026-08-05T02:00:00Z" },
  );
  assert.equal(evaluation.execution.replayOf, replay.experimentId);
  assert.equal(evaluation.result.outcome, "rejected");
  assert.deepEqual(evaluation.relations.contradicts, [replay.experimentId]);
  assert.equal(evaluation.evaluation.evaluator, "owner-independent");
});

test("refuses to validate a replay while an execution layer is missing", () => {
  const replay = {
    ...candidate,
    experimentId: "replay-1",
    execution: { ...candidate.execution, replayOf: "proposal-1" },
  };
  assert.throws(
    () =>
      buildIndependentEvaluation(
        replay,
        {
          outcome: "validated",
          evidenceIds: "observer:1",
          rationale: "The observed portion worked.",
        },
        { experimentId: "evaluation-1", recordedAt: "2026-08-05T02:00:00Z" },
      ),
    /missing coverage cannot be validated/,
  );
});
