import assert from "node:assert/strict";
import test from "node:test";
import { mergeBwIssues } from "./bwProjection.ts";
import { eventToProjectIssue } from "./projectIssues.mjs";

const root = {
  id: "a".repeat(64),
  pubkey: "b".repeat(64),
  created_at: 1,
  kind: 1621,
  content: "Original",
  tags: [
    ["a", `30617:${"b".repeat(64)}:repo`],
    ["subject", "Issue"],
  ],
};
function snapshot(state = "resolved") {
  return {
    repo: root.tags[0][1],
    activation: null,
    records: { [root.id]: root },
    decisions: {},
    notices: {},
    projection: {
      issues: { [root.id]: state },
      issue_fields: {},
      conflicts: [],
      children: {},
      relations: [],
      artifact_verdicts: {},
    },
  };
}
test("Core acceptance replaces legacy status and survives a legacy tombstone-filtered list", () => {
  const legacy = {
    ...eventToProjectIssue(root),
    status: "Backlog",
    currentReview: { id: "legacy" },
  };
  for (const rows of [[], [legacy]]) {
    const [issue] = mergeBwIssues(rows, snapshot());
    assert.equal(issue.status, "Done");
    assert.equal(issue.currentReview, null);
    assert.equal(issue.workflowStatus, null);
    assert.equal(issue.bw.enrolled, true);
  }
});
test("Pending enrollment never adopts legacy workflow authority", () => {
  const s = snapshot();
  s.projection.issues = {};
  s.notices[root.id] = [
    {
      event_id: "pending",
      outcome: "pending",
      stage: "references",
      code: "missing-reference",
    },
  ];
  const [issue] = mergeBwIssues(
    [{ ...eventToProjectIssue(root), status: "Done" }],
    s,
  );
  assert.equal(issue.status, "Triage");
  assert.equal(issue.bw.enrolled, false);
  assert.match(issue.bw.nextActor, /Waiting/);
});
test("Technical conflict is visible without reopening accepted history", () => {
  const s = snapshot();
  s.notices[root.id] = [
    { event_id: "fork", outcome: "conflict", stage: "causality", code: "fork" },
  ];
  assert.equal(mergeBwIssues([], s)[0].status, "Done");
  assert.equal(mergeBwIssues([], s)[0].bw.notices[0].outcome, "conflict");
});
test("Unenrolled legacy issues stay usable and foreign/unverified roots cannot be added", () => {
  const legacy = eventToProjectIssue(root);
  const s = snapshot();
  s.records = {};
  assert.deepEqual(mergeBwIssues([legacy], s), [legacy]);
  assert.deepEqual(mergeBwIssues([], s), []);
});
