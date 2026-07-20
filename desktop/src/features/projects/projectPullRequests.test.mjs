import assert from "node:assert/strict";
import test from "node:test";

import {
  eventToProjectPullRequest,
  nextProjectPullRequestReviewCreatedAt,
  nextProjectPullRequestStatusCreatedAt,
  projectPullRequestCommentTimelineKind,
  projectPullRequestEffectiveReviewDecision,
  projectPullRequestReviewSummary,
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
      ["branch-name", "feature/demo"],
      ["target-branch", "release"],
      ["clone", `https://relay.example/git/${OWNER}/demo`],
    ],
    ...overrides,
  };
}

test("reads source and target branches from the pull request", () => {
  const pullRequest = eventToProjectPullRequest(pullRequestEvent());

  assert.equal(pullRequest.branchName, "feature/demo");
  assert.equal(pullRequest.targetBranch, "release");
});

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
  id,
  pubkey,
  createdAt,
  content = "",
  labels = [],
  reviewers = [],
  commit,
  anchor,
}) {
  return {
    id: id ?? `comment-${pubkey.slice(0, 8)}-${createdAt}`,
    kind: 1,
    pubkey,
    created_at: createdAt,
    content,
    tags: [
      ["e", "f".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ...reviewers.map((reviewer) => ["p", reviewer]),
      ...labels.map((label) => ["t", label]),
      ...(commit ? [["c", commit]] : []),
      ...(anchor
        ? [
            ["t", "inline-comment"],
            ["file", anchor.path],
            ["side", anchor.side],
            ["line", String(anchor.line)],
          ]
        : []),
    ],
  };
}

test("parses commit-scoped inline comment anchors", () => {
  const commit = "1111111111111111111111111111111111111111";
  const comment = commentEvent({
    pubkey: ATTACKER,
    createdAt: 150,
    content: "Could this be clearer?",
    commit,
    anchor: {
      path: "desktop/src/features/projects/hooks.ts",
      side: "new",
      line: 42,
    },
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [comment],
  );

  assert.deepEqual(pullRequest.comments[0].anchor, {
    path: "desktop/src/features/projects/hooks.ts",
    side: "new",
    line: 42,
  });
  assert.equal(pullRequest.comments[0].inlineCommentStatus, "current");
  assert.equal(pullRequest.comments[0].isInlineComment, true);
});

test("marks inline comments on earlier commits as outdated", () => {
  const initialCommit = "1111111111111111111111111111111111111111";
  const currentCommit = "2222222222222222222222222222222222222222";
  const comment = commentEvent({
    pubkey: ATTACKER,
    createdAt: 150,
    commit: initialCommit,
    anchor: { path: "src/example.ts", side: "old", line: 7 },
  });
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: currentCommit,
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [update],
    [comment],
  );

  assert.equal(pullRequest.comments[0].inlineCommentStatus, "outdated");
});

test("rejects malformed inline comment anchors", () => {
  const comment = commentEvent({
    pubkey: ATTACKER,
    createdAt: 150,
    commit: "1111111111111111111111111111111111111111",
    anchor: { path: "../secrets", side: "new", line: 0 },
  });

  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [comment],
  );

  assert.equal(pullRequest.comments[0].anchor, null);
  assert.equal(pullRequest.comments[0].inlineCommentStatus, null);
});

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
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === AUTHOR)
      ?.isTrustedReviewRequest,
    true,
  );
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === ATTACKER)
      ?.isTrustedReviewRequest,
    false,
  );
});

test("approvals keep the latest per author and flag comments", () => {
  const reviewer = "d".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
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
    event,
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

test("review decisions only apply to the current pull request commit", () => {
  const reviewer = "d".repeat(64);
  const initialCommit = "1111111111111111111111111111111111111111";
  const currentCommit = "2222222222222222222222222222222222222222";
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 300,
    commit: currentCommit,
  });
  const staleApproval = commentEvent({
    pubkey: reviewer,
    createdAt: 200,
    labels: ["approval"],
    commit: initialCommit,
  });
  const currentChangeRequest = commentEvent({
    pubkey: reviewer,
    createdAt: 400,
    labels: ["changes-requested"],
    commit: currentCommit,
  });

  const pullRequest = eventToProjectPullRequest(
    event,
    [update],
    [staleApproval, currentChangeRequest],
  );

  assert.deepEqual(pullRequest.approvals, []);
  assert.equal(pullRequest.changeRequests.length, 1);
  assert.equal(pullRequest.changeRequests[0].author, reviewer);
  assert.equal(pullRequest.changeRequests[0].commit, currentCommit);
  assert.equal(
    pullRequest.comments.find((comment) => comment.id === staleApproval.id)
      ?.reviewDecisionStatus,
    "historical",
  );
  assert.equal(
    pullRequest.comments.find(
      (comment) => comment.id === currentChangeRequest.id,
    )?.reviewDecisionStatus,
    "current",
  );
});

test("the latest current-commit review decision replaces the previous state", () => {
  const reviewer = "d".repeat(64);
  const commit = "1111111111111111111111111111111111111111";
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const changeRequest = commentEvent({
    pubkey: reviewer,
    createdAt: 200,
    labels: ["changes-requested"],
    commit,
  });
  const approval = commentEvent({
    pubkey: reviewer,
    createdAt: 300,
    labels: ["approval"],
    commit,
  });

  const pullRequest = eventToProjectPullRequest(
    event,
    [],
    [changeRequest, approval],
  );

  assert.equal(pullRequest.approvals.length, 1);
  assert.equal(pullRequest.approvals[0].author, reviewer);
  assert.deepEqual(pullRequest.changeRequests, []);
  assert.equal(
    pullRequest.comments.filter((comment) => comment.isChangeRequest).length,
    1,
  );
  assert.equal(nextProjectPullRequestReviewCreatedAt(pullRequest, 300), 301);
  assert.equal(nextProjectPullRequestReviewCreatedAt(pullRequest, 400), 400);
});

test("effective review decisions exclude superseded and untrusted labels", () => {
  const reviewer = "d".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const supersededApproval = commentEvent({
    pubkey: reviewer,
    createdAt: 200,
    labels: ["approval"],
  });
  const effectiveChangeRequest = commentEvent({
    pubkey: reviewer,
    createdAt: 300,
    labels: ["changes-requested"],
  });
  const effectiveApproval = commentEvent({
    pubkey: OWNER,
    createdAt: 350,
    labels: ["approval"],
  });
  const untrustedApproval = commentEvent({
    pubkey: ATTACKER,
    createdAt: 400,
    labels: ["approval"],
  });
  const pullRequest = eventToProjectPullRequest(
    event,
    [],
    [
      supersededApproval,
      effectiveChangeRequest,
      effectiveApproval,
      untrustedApproval,
    ],
  );
  const commentById = new Map(
    pullRequest.comments.map((comment) => [comment.id, comment]),
  );

  assert.equal(
    projectPullRequestEffectiveReviewDecision(
      pullRequest,
      commentById.get(effectiveChangeRequest.id),
    ),
    "changes-requested",
  );
  assert.equal(
    projectPullRequestEffectiveReviewDecision(
      pullRequest,
      commentById.get(effectiveApproval.id),
    ),
    "approved",
  );
  assert.equal(
    projectPullRequestEffectiveReviewDecision(
      pullRequest,
      commentById.get(supersededApproval.id),
    ),
    null,
  );
  assert.equal(
    projectPullRequestEffectiveReviewDecision(
      pullRequest,
      commentById.get(untrustedApproval.id),
    ),
    null,
  );
});

test("mixed effective decisions keep requested changes visible", () => {
  const reviewerOne = "d".repeat(64);
  const reviewerTwo = "e".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewerOne], ["p", reviewerTwo]);
  const pullRequest = eventToProjectPullRequest(
    event,
    [],
    [
      commentEvent({
        pubkey: reviewerOne,
        createdAt: 200,
        labels: ["approval"],
      }),
      commentEvent({
        pubkey: reviewerTwo,
        createdAt: 300,
        labels: ["changes-requested"],
      }),
    ],
  );

  const summary = projectPullRequestReviewSummary(pullRequest);

  assert.equal(summary.approvalCount, 1);
  assert.equal(summary.changeRequestCount, 1);
  assert.equal(summary.showState, true);
  assert.equal(summary.state, "1 reviewer requested changes.");
});

test("trusted review requests cannot borrow decision timeline visuals", () => {
  const reviewer = "d".repeat(64);
  const request = commentEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    labels: ["review-request", "approval"],
    reviewers: [reviewer],
  });
  const pullRequest = eventToProjectPullRequest(
    pullRequestEvent(),
    [],
    [request],
  );
  const comment = pullRequest.comments[0];

  assert.equal(comment.isTrustedReviewRequest, true);
  assert.equal(comment.isTrustedReviewDecision, false);
  assert.equal(
    projectPullRequestCommentTimelineKind(comment),
    "review-request",
  );
});

test("equal-timestamp decisions use event ID ordering in both input orders", () => {
  const reviewer = "d".repeat(64);
  const commit = "1111111111111111111111111111111111111111";
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const approval = commentEvent({
    id: "a".repeat(64),
    pubkey: reviewer,
    createdAt: 300,
    labels: ["approval"],
    commit,
  });
  const changeRequest = commentEvent({
    id: "b".repeat(64),
    pubkey: reviewer,
    createdAt: 300,
    labels: ["changes-requested"],
    commit,
  });

  for (const comments of [
    [approval, changeRequest],
    [changeRequest, approval],
  ]) {
    const pullRequest = eventToProjectPullRequest(event, [], comments);

    assert.deepEqual(pullRequest.approvals, []);
    assert.equal(pullRequest.changeRequests.length, 1);
    assert.equal(pullRequest.changeRequests[0].id, changeRequest.id);
  }
});

test("legacy unscoped review decisions are invalidated by later PR updates", () => {
  const reviewer = "d".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const approval = commentEvent({
    pubkey: reviewer,
    createdAt: 200,
    labels: ["approval"],
  });
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 300,
    commit: "2222222222222222222222222222222222222222",
  });

  const pullRequest = eventToProjectPullRequest(event, [update], [approval]);

  assert.deepEqual(pullRequest.approvals, []);
  assert.deepEqual(pullRequest.changeRequests, []);
  assert.equal(
    pullRequest.comments.find((comment) => comment.id === approval.id)
      ?.reviewDecisionStatus,
    "historical",
  );
});

test("legacy decisions stay bound to the initial commit after a backdated update", () => {
  const reviewer = "d".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const update = updateEvent({
    pubkey: AUTHOR,
    createdAt: 200,
    commit: "2222222222222222222222222222222222222222",
  });
  const approval = commentEvent({
    pubkey: reviewer,
    createdAt: 300,
    labels: ["approval"],
  });

  const pullRequest = eventToProjectPullRequest(event, [update], [approval]);

  assert.deepEqual(pullRequest.approvals, []);
  assert.equal(
    pullRequest.comments.find((comment) => comment.id === approval.id)
      ?.reviewDecisionStatus,
    "historical",
  );
});

test("approvals only count requested reviewers and the repository owner", () => {
  const reviewer = "d".repeat(64);
  const event = pullRequestEvent();
  event.tags.push(["p", reviewer]);
  const comments = [
    commentEvent({
      pubkey: reviewer,
      createdAt: 200,
      labels: ["approval"],
    }),
    commentEvent({
      pubkey: OWNER,
      createdAt: 210,
      labels: ["approval"],
    }),
    commentEvent({
      pubkey: ATTACKER,
      createdAt: 220,
      labels: ["approval"],
    }),
    commentEvent({
      pubkey: AUTHOR,
      createdAt: 230,
      labels: ["approval"],
    }),
  ];

  const pullRequest = eventToProjectPullRequest(event, [], comments);

  assert.deepEqual(
    pullRequest.approvals.map((approval) => approval.author),
    [reviewer, OWNER],
  );
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === reviewer)
      ?.isTrustedReviewDecision,
    true,
  );
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === reviewer)
      ?.reviewDecisionStatus,
    "current",
  );
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === ATTACKER)
      ?.isTrustedReviewDecision,
    false,
  );
  assert.equal(
    pullRequest.comments.find((comment) => comment.author === ATTACKER)
      ?.reviewDecisionStatus,
    null,
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
