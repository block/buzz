import assert from "node:assert/strict";
import test from "node:test";

import { projectBriefDecisions } from "./briefDecisions.ts";

const proposal = {
  classification: "OFFICIAL",
  actionId: "readiness-1",
  text: "Complete the readiness review today.",
  alternativeText:
    "Defer the review until tomorrow and retain the current posture.",
  approvalState: "pending",
  sourceIds: ["ledger-1"],
};

function brief(proposedActions = [proposal]) {
  return {
    runId: "run-1",
    sections: {
      decisions: [
        {
          classification: "OFFICIAL",
          text: "Complete the readiness review today.",
          sourceIds: ["ledger-1"],
        },
      ],
    },
    contributions: [
      { adviser: "operations", proposedActions },
      { adviser: "plans", proposedActions },
    ],
  };
}

test("projects one concise two-course decision without exposing sources", () => {
  const decisions = projectBriefDecisions(brief());
  assert.deepEqual(decisions, [
    {
      key: "run-1:readiness-1",
      runId: "run-1",
      actionId: "readiness-1",
      adviser: "operations",
      coaA: "Complete the readiness review today.",
      coaB: "Defer the review until tomorrow and retain the current posture.",
    },
  ]);
  assert.equal("sourceIds" in decisions[0], false);
});

test("keeps prior briefs actionable without inventing a second course", () => {
  const decisions = projectBriefDecisions(
    brief([{ ...proposal, alternativeText: undefined }]),
  );
  assert.equal(decisions.length, 1);
  assert.equal(decisions[0].coaB, undefined);
});

test("does not surface a proposal absent from the validated decisions section", () => {
  const value = brief();
  value.sections.decisions = [];
  assert.deepEqual(projectBriefDecisions(value), []);
});
