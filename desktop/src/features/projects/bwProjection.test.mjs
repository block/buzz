import assert from "node:assert/strict";
import test from "node:test";
import {
  bwChainHead,
  bwFieldConflict,
  bwMatchingTriageDelegation,
  mergeBwIssues,
} from "./bwProjection.ts";
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
test("Core ready projects as Ready instead of remaining in Backlog", () => {
  const [issue] = mergeBwIssues([], snapshot("ready"));
  assert.equal(issue.status, "Ready");
  assert.equal(issue.bw.state, "ready");
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

test("A BW-shaped root with no enroll attempt yet is clearly not a BW issue", () => {
  const s = snapshot();
  s.projection.issues = {};
  const [issue] = mergeBwIssues([], s);
  assert.equal(issue.bw.enrolled, false);
  assert.equal(issue.bw.notices.length, 0);
  assert.match(issue.bw.nextActor, /Not yet a BW issue/);
});

function triageActionRecord(id, action, previous) {
  const tags = [
    ["record", "triage-action"],
    ["a", root.tags[0][1]],
    ["policy", "p".repeat(64)],
    ["issue", root.id],
  ];
  if (previous) tags.push(["previous", previous]);
  return {
    id,
    pubkey: "c".repeat(64),
    created_at: 2,
    kind: 46100,
    tags,
    content: JSON.stringify(
      action === "need-info"
        ? { action, question: "q", recipient: "d".repeat(64) }
        : { action },
    ),
  };
}

test("A need-info triage head surfaces as Needs Clarification, not plain Triage", () => {
  const s = snapshot("triage");
  const needInfo = triageActionRecord("1".repeat(64), "need-info");
  s.records[needInfo.id] = needInfo;
  const [issue] = mergeBwIssues([], s);
  assert.equal(issue.status, "Needs Clarification");
  assert.match(issue.bw.nextActor, /clarification/);
});

test("bwChainHead reports a conflict instead of guessing a previous on a fork", () => {
  const s = snapshot("triage");
  const forkA = triageActionRecord("1".repeat(64), "need-info");
  const forkB = triageActionRecord("2".repeat(64), "need-info");
  s.records[forkA.id] = forkA;
  s.records[forkB.id] = forkB;
  const { headId, conflict } = bwChainHead(s, root.id, "triage-action");
  assert.equal(conflict, true);
  assert.equal(headId, null);
  // A fork among triage-action records does not surface as Needs
  // Clarification: only a single, unconflicted need-info head does.
  const [issue] = mergeBwIssues([], s);
  assert.equal(issue.status, "Triage");
});

test("bwFieldConflict flags an issue whose issue-update chain forked", () => {
  const s = snapshot("triage");
  s.projection.conflicts = ["1".repeat(64), "2".repeat(64)];
  s.records["1".repeat(64)] = {
    id: "1".repeat(64),
    pubkey: "c".repeat(64),
    created_at: 2,
    kind: 46100,
    tags: [
      ["record", "issue-update"],
      ["a", root.tags[0][1]],
      ["policy", "p".repeat(64)],
      ["issue", root.id],
    ],
    content: '{"patch":{"title":"A"}}',
  };
  assert.equal(bwFieldConflict(s, root.id), true);
  assert.equal(bwFieldConflict(s, "unrelated-issue"), false);
});

function policyRecord(id, triageDelegations) {
  return {
    id,
    pubkey: "b".repeat(64),
    created_at: 1,
    kind: 46100,
    tags: [["record", "role-policy"]],
    content: JSON.stringify({ triage_delegations: triageDelegations }),
  };
}

test("bwMatchingTriageDelegation matches the exact issue/action/signer pair before expiry", () => {
  const s = snapshot("triage");
  const policyId = "p".repeat(64);
  s.activation = { policy: policyId, genesis: "g".repeat(64) };
  s.records[policyId] = policyRecord(policyId, [
    {
      action: "accept",
      delegate: "e".repeat(64),
      expires_at: 1000,
      issue: root.id,
    },
  ]);
  assert.equal(
    bwMatchingTriageDelegation(s, root.id, "accept", "e".repeat(64), 500),
    true,
  );
});

test("bwMatchingTriageDelegation is false with no matching delegation entry", () => {
  const s = snapshot("triage");
  const policyId = "p".repeat(64);
  s.activation = { policy: policyId, genesis: "g".repeat(64) };
  s.records[policyId] = policyRecord(policyId, []);
  assert.equal(
    bwMatchingTriageDelegation(s, root.id, "accept", "e".repeat(64), 500),
    false,
  );
  // No activated policy at all behaves the same way.
  assert.equal(
    bwMatchingTriageDelegation(
      snapshot("triage"),
      root.id,
      "accept",
      "e".repeat(64),
      500,
    ),
    false,
  );
});

test("bwMatchingTriageDelegation rejects an expired or another signer's delegation", () => {
  const s = snapshot("triage");
  const policyId = "p".repeat(64);
  s.activation = { policy: policyId, genesis: "g".repeat(64) };
  s.records[policyId] = policyRecord(policyId, [
    {
      action: "accept",
      delegate: "e".repeat(64),
      expires_at: 100,
      issue: root.id,
    },
  ]);
  assert.equal(
    bwMatchingTriageDelegation(s, root.id, "accept", "e".repeat(64), 500),
    false,
  );
  s.records[policyId] = policyRecord(policyId, [
    {
      action: "accept",
      delegate: "e".repeat(64),
      expires_at: 1000,
      issue: root.id,
    },
  ]);
  assert.equal(
    bwMatchingTriageDelegation(s, root.id, "accept", "f".repeat(64), 500),
    false,
  );
});
