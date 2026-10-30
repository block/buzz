import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveEvalPromotionDecision,
  deriveEvalVerdict,
} from "./evalVerdict.ts";

const completeRun = {
  agentPubkey: "agent-pubkey",
  evaluatorPubkey: "evaluator-pubkey",
  criteria: {
    id: "refund-created",
    version: 1,
    ownerPubkey: "owner-pubkey",
    lockedBeforeRun: true,
  },
  evidence: [
    { id: "database-receipt", kind: "outcome", source: "database readback" },
    { id: "observer-trace", kind: "process", source: "observer transcript" },
  ],
  graderPassed: true,
};

test("validates only when outcome and process evidence are both present", () => {
  assert.deepEqual(deriveEvalVerdict(completeRun), {
    status: "validated",
    missingCoverage: [],
    evidenceIds: ["database-receipt", "observer-trace"],
  });

  const verdict = deriveEvalVerdict({
    ...completeRun,
    evidence: completeRun.evidence.filter(({ kind }) => kind !== "outcome"),
  });
  assert.equal(verdict.status, "inconclusive");
  assert.deepEqual(verdict.missingCoverage, ["outcome evidence"]);
});

test("refuses self-grading and criteria changed after the run began", () => {
  const verdict = deriveEvalVerdict({
    ...completeRun,
    evaluatorPubkey: completeRun.agentPubkey,
    criteria: { ...completeRun.criteria, lockedBeforeRun: false },
  });

  assert.equal(verdict.status, "inconclusive");
  assert.deepEqual(verdict.missingCoverage, [
    "criteria locked before run",
    "independent evaluator",
  ]);
});

test("records a supported failure as rejected rather than inconclusive", () => {
  const verdict = deriveEvalVerdict({ ...completeRun, graderPassed: false });
  assert.equal(verdict.status, "rejected");
  assert.deepEqual(verdict.missingCoverage, []);
});

test("promotion requires validation, non-regression, and owner approval", () => {
  const verdict = deriveEvalVerdict(completeRun);

  assert.deepEqual(
    deriveEvalPromotionDecision({
      verdict,
      baselinePassRate: 0.8,
      candidatePassRate: 0.9,
      ownerApproved: true,
    }),
    { allowed: true, reasons: [] },
  );

  assert.deepEqual(
    deriveEvalPromotionDecision({
      verdict,
      baselinePassRate: 0.9,
      candidatePassRate: 0.8,
      ownerApproved: false,
    }),
    {
      allowed: false,
      reasons: [
        "candidate regresses against baseline",
        "owner approval is missing",
      ],
    },
  );
});
