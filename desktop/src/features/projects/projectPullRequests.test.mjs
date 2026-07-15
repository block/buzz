import assert from "node:assert/strict";
import test from "node:test";

import {
  eventToProjectPullRequest,
  nextProjectPullRequestStatusCreatedAt,
} from "./projectPullRequests.mjs";

const OWNER = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const ATTACKER = "c".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:demo`;

function pullRequestEvent(overrides = {}) {
  return {
    id: "f".repeat(64),
    kind: 1618,
    pubkey: AUTHOR,
    created_at: 100,
    content: "Add feature\n\nDetails.",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Add feature"],
      ["c", "1111111111111111111111111111111111111111"],
      ["clone", `https://relay.example/git/${OWNER}/demo`],
    ],
    ...overrides,
  };
}

function updateEvent({ pubkey, createdAt, commit, cloneUrl }) {
  return {
    id: `update-${pubkey.slice(0, 8)}-${createdAt}`,
    kind: 1619,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: [
      ["E", "f".repeat(64)],
      ["a", REPO_ADDRESS],
      ["c", commit],
      ...(cloneUrl ? [["clone", cloneUrl]] : []),
    ],
  };
}

function statusEvent({ kind, pubkey, createdAt }) {
  return {
    id: `status-${pubkey.slice(0, 8)}-${createdAt}`,
    kind,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: [
      ["e", "f".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

test("accepts updates signed by the PR author", () => {
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: "2222222222222222222222222222222222222222",
    cloneUrl: `https://relay.example/git/${AUTHOR}/demo-fork`,
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [update]);

  assert.equal(pullRequest.commit, "2222222222222222222222222222222222222222");
  assert.deepEqual(pullRequest.cloneUrls, [
    `https://relay.example/git/${AUTHOR}/demo-fork`,
  ]);
  assert.equal(pullRequest.updateCount, 1);
});

test("accepts updates signed by the repo owner", () => {
  const update = updateEvent({
    pubkey: OWNER,
    createdAt: 200,
    commit: "3333333333333333333333333333333333333333",
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [update]);

  assert.equal(pullRequest.commit, "3333333333333333333333333333333333333333");
});

test("ignores a later update from a different pubkey", () => {
  const authorUpdate = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: "2222222222222222222222222222222222222222",
    cloneUrl: `https://relay.example/git/${OWNER}/demo`,
  });
  const attackerUpdate = updateEvent({
    pubkey: ATTACKER,
    createdAt: 300,
    commit: "6666666666666666666666666666666666666666",
    cloneUrl: "https://evil.example/git/attacker/repo",
  });

  const pullRequest = eventToProjectPullRequest(pullRequestEvent(), [
    authorUpdate,
    attackerUpdate,
  ]);

  assert.equal(pullRequest.commit, "2222222222222222222222222222222222222222");
  assert.deepEqual(pullRequest.cloneUrls, [
    `https://relay.example/git/${OWNER}/demo`,
  ]);
  assert.equal(pullRequest.updateCount, 1);
});

test("ignores status events from a different pubkey", () => {
  const attackerMerged = statusEvent({
    kind: 1631,
    pubkey: ATTACKER,
    createdAt: 300,
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [attackerMerged],
  );

  assert.equal(pullRequest.status, "Open");
});

test("honors status events from the PR author and repo owner", () => {
  const authorMerged = statusEvent({
    kind: 1631,
    pubkey: AUTHOR,
    createdAt: 300,
  });
  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [authorMerged],
  );
  assert.equal(pullRequest.status, "Merged");

  const ownerClosed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 400,
  });
  const closedPullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [ownerClosed],
  );
  assert.equal(closedPullRequest.status, "Closed");
});

function commentEvent({
  pubkey,
  createdAt,
  content = "",
  labels = [],
  reviewers = [],
}) {
  return {
    id: `comment-${pubkey.slice(0, 8)}-${createdAt}`,
    kind: 1,
    pubkey,
    created_at: createdAt,
    content,
    tags: [
      ["e", "f".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ...reviewers.map((reviewer) => ["p", reviewer]),
      ...labels.map((label) => ["t", label]),
    ],
  };
}

test("draft/ready toggles via status kinds 1633 and 1630", () => {
  const draft = statusEvent({ kind: 1633, pubkey: AUTHOR, createdAt: 300 });
  const draftPullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [draft],
  );
  assert.equal(draftPullRequest.status, "Draft");
  assert.equal(draftPullRequest.statusCreatedAt, 300);
  assert.equal(
    nextProjectPullRequestStatusCreatedAt(draftPullRequest, 300),
    301,
  );
  assert.equal(
    nextProjectPullRequestStatusCreatedAt(draftPullRequest, 400),
    400,
  );

  const reopened = statusEvent({ kind: 1630, pubkey: AUTHOR, createdAt: 400 });
  const openPullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [],
    [draft, reopened],
  );
  assert.equal(openPullRequest.status, "Open");
});

test("reviewers come from root p tags plus trusted review requests", () => {
  const reviewer = "d".repeat(64);
  const requested = "e".repeat(64);
  const event = pullRequestEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Add feature"],
      ["c", "1111111111111111111111111111111111111111"],
      ["p", reviewer],
      ["p", AUTHOR],
    ],
  });
  const request = commentEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    content: "Requested a review from someone",
    labels: ["review-request"],
    reviewers: [requested],
  });
  const untrustedRequest = commentEvent({
    pubkey: ATTACKER,
    createdAt: 300,
    labels: ["review-request"],
    reviewers: ["9".repeat(64)],
  });

  const pullRequest = eventToProjectPullRequest(
    event,
    [],
    [request, untrustedRequest],
  );

  // The author never reviews their own PR; untrusted requests are ignored.
  assert.deepEqual(pullRequest.reviewers.sort(), [reviewer, requested].sort());
});

test("approvals keep the latest per author and flag comments", () => {
  const reviewer = "d".repeat(64);
  const firstApproval = commentEvent({
    pubkey: reviewer,
    createdAt: 200,
    content: "Approved these changes",
    labels: ["approval"],
  });
  const secondApproval = commentEvent({
    pubkey: reviewer,
    createdAt: 300,
    content: "Approved these changes",
    labels: ["approval"],
  });
  const plainComment = commentEvent({
    pubkey: OWNER,
    createdAt: 250,
    content: "Looks good",
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [firstApproval, plainComment, secondApproval],
  );

  assert.equal(pullRequest.approvals.length, 1);
  assert.equal(pullRequest.approvals[0].author, reviewer);
  assert.equal(pullRequest.approvals[0].createdAt, 300);
  assert.equal(
    pullRequest.comments.filter((comment) => comment.isApproval).length,
    2,
  );
  assert.equal(
    pullRequest.comments.filter((comment) => comment.isReviewRequest).length,
    0,
  );
});

test("survives malformed value-less tags", () => {
  const event = pullRequestEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["t"],
      ["p"],
      ["c", "1111111111111111111111111111111111111111"],
    ],
  });

  const pullRequest = eventToProjectPullRequest(event);

  assert.equal(pullRequest.status, "Open");
  assert.deepEqual(pullRequest.labels, []);
  assert.deepEqual(pullRequest.recipients, []);
});
