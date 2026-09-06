import assert from "node:assert/strict";
import test from "node:test";

import {
  buildMyBuzzWorkflowStatus,
  buildProjectIssueVerdict,
  canSubmitProjectIssueVerdict,
  canWriteMyBuzzWorkflowStatus,
  MAX_REJECTION_REASON_LENGTH,
} from "./issueStatus.ts";

const OWNER = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const ASSIGNEE = "d".repeat(64);
const OA_OWNER = "e".repeat(64);

test("builds the exact owner workflow status envelope", () => {
  const issue = { id: "f".repeat(64), statusCreatedAt: null };
  const project = { repoAddress: `30617:${OWNER}:mybuzz` };
  assert.deepEqual(
    buildMyBuzzWorkflowStatus({
      issue,
      project,
      reason: "Ready for human testing.",
      state: "ready-for-test",
    }),
    {
      content: "Status changed to ready-for-test.",
      kind: 1,
      tags: [
        ["e", issue.id, "", "root"],
        ["a", project.repoAddress],
        ["t", "mybuzz-workflow-status"],
        ["workflow", "mybuzz-status-v1"],
        ["state", "ready-for-test"],
        ["reason", "Ready for human testing."],
      ],
    },
  );
  assert.throws(
    () => buildMyBuzzWorkflowStatus({ issue, project, state: "triage" }),
    /reason/i,
  );
  assert.throws(
    () =>
      buildMyBuzzWorkflowStatus({
        issue,
        project,
        reason: "bad\nreason",
        state: "implemented",
      }),
    /reason/i,
  );
  assert.throws(
    () =>
      buildMyBuzzWorkflowStatus({
        issue,
        project,
        reason: " padded ",
        state: "implemented",
      }),
    /reason/i,
  );
});

test("only the fixed MyBuzz owner can write workflow status", () => {
  assert.equal(canWriteMyBuzzWorkflowStatus(OWNER), false);
  assert.equal(
    canWriteMyBuzzWorkflowStatus(
      "dd57c78422bccf568feb2a7ae5bcf4d7ebefc2c6c54bf56a26faeb9e0b08d36b".toUpperCase(),
    ),
    true,
  );
  assert.equal(canWriteMyBuzzWorkflowStatus(null), false);
});

test("builds exact personal-signed human verdict contracts", () => {
  const reviewId = "9".repeat(64);
  const issue = {
    author: AUTHOR,
    currentReview: { id: reviewId },
    id: "f".repeat(64),
  };
  const project = { owner: OWNER, repoAddress: `30617:${OWNER}:demo` };

  assert.deepEqual(
    buildProjectIssueVerdict({ issue, project, verdict: "accepted" }),
    {
      content: "",
      kind: 1631,
      tags: [
        ["e", issue.id, "", "root"],
        ["a", project.repoAddress],
        ["p", OWNER],
        ["p", AUTHOR],
        ["t", "human-verdict"],
        ["verdict", "accepted"],
        ["review", reviewId],
      ],
    },
  );
  assert.deepEqual(
    buildProjectIssueVerdict({
      issue,
      project,
      reason: "The target does not load.",
      verdict: "rejected",
    }),
    {
      content: "The target does not load.",
      kind: 1630,
      tags: [
        ["e", issue.id, "", "root"],
        ["a", project.repoAddress],
        ["p", OWNER],
        ["p", AUTHOR],
        ["t", "human-verdict"],
        ["verdict", "rejected"],
        ["review", reviewId],
      ],
    },
  );
  assert.throws(
    () =>
      buildProjectIssueVerdict({
        issue,
        project,
        reason: "bad\nreason",
        verdict: "rejected",
      }),
    /reason/i,
  );
});

test("human verdict recipients are the deduped exact owner and author set", () => {
  const issue = {
    author: OWNER,
    currentReview: { id: "9".repeat(64) },
    id: "f".repeat(64),
  };
  const project = { owner: OWNER, repoAddress: `30617:${OWNER}:demo` };

  const verdict = buildProjectIssueVerdict({
    issue,
    project,
    verdict: "accepted",
  });
  assert.deepEqual(
    verdict.tags.filter((tag) => tag[0] === "p"),
    [["p", OWNER]],
  );
  assert.equal(verdict.content, "");
});

test("rejection reasons enforce the exact 500-character contract", () => {
  assert.equal(MAX_REJECTION_REASON_LENGTH, 500);
  const issue = {
    author: AUTHOR,
    currentReview: { id: "9".repeat(64) },
    id: "f".repeat(64),
  };
  const project = { owner: OWNER, repoAddress: `30617:${OWNER}:demo` };

  assert.equal(
    buildProjectIssueVerdict({
      issue,
      project,
      reason: "x".repeat(500),
      verdict: "rejected",
    }).content.length,
    500,
  );
  assert.throws(
    () =>
      buildProjectIssueVerdict({
        issue,
        project,
        reason: "x".repeat(501),
        verdict: "rejected",
      }),
    /500/,
  );
});

test("only configured human verdict actors may submit a current review", () => {
  const issue = {
    currentReview: { authorizedHumanPubkeys: [OWNER, OA_OWNER] },
  };
  assert.equal(canSubmitProjectIssueVerdict(issue, OWNER), true);
  assert.equal(
    canSubmitProjectIssueVerdict(issue, OA_OWNER.toUpperCase()),
    true,
  );
  assert.equal(canSubmitProjectIssueVerdict(issue, ASSIGNEE), false);
  assert.equal(canSubmitProjectIssueVerdict(issue, null), false);
  assert.equal(
    canSubmitProjectIssueVerdict({ currentReview: null }, OWNER),
    false,
  );
});
