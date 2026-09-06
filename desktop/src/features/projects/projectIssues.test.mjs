import assert from "node:assert/strict";
import test from "node:test";

import {
  buildGitIssueTags,
  eventToProjectIssue,
  getAllTags,
  getTag,
  isHumanDirectedIssueComment,
  ISSUE_ACTION_REQUIRED_LABEL,
  ISSUE_ASSIGNMENT_LABEL,
  ISSUE_UNASSIGNMENT_LABEL,
  nextProjectIssueCommentCreatedAt,
  nextProjectIssueStatusCreatedAt,
  PROJECT_ISSUE_STATUS,
} from "./projectIssues.mjs";

const OWNER =
  "dd57c78422bccf568feb2a7ae5bcf4d7ebefc2c6c54bf56a26faeb9e0b08d36b";
const AUTHOR = "b".repeat(64);
const ATTACKER = "c".repeat(64);
const OA_OWNER = "d".repeat(64);
const REVIEW_TESTER = "f".repeat(64);
const REVIEW_COORDINATOR = "1".repeat(64);
const REVIEW_ID = "9".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:demo`;

function workflowStatusEvent({
  id = "a".repeat(64),
  state = "triage",
  reason,
  pubkey = OWNER,
  createdAt = 200,
  tags = [],
  ...overrides
} = {}) {
  return {
    id,
    kind: 1,
    pubkey,
    created_at: createdAt,
    content: `Status changed to ${state}.`,
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "mybuzz-workflow-status"],
      ["workflow", "mybuzz-status-v1"],
      ["state", state],
      ...(reason === undefined ? [] : [["reason", reason]]),
      ...tags,
    ],
    ...overrides,
  };
}

test("renders the latest valid owner workflow status independently of NIP-34", () => {
  const nativeResolved = statusEvent({
    kind: 1631,
    pubkey: OWNER,
    createdAt: 300,
  });
  const triage = workflowStatusEvent({
    id: "a".repeat(64),
    reason: "Initial classification.",
    createdAt: 100,
  });
  const implemented = workflowStatusEvent({
    id: "b".repeat(64),
    state: "implemented",
    createdAt: 100,
  });

  const issue = eventToProjectIssue(
    issueEvent(),
    [nativeResolved],
    [triage, implemented, implemented],
  );

  assert.equal(issue.status, "Implemented");
  assert.equal(issue.workflowStatus?.eventId, implemented.id);
  assert.equal(issue.workflowStatus?.reason, null);
});

test("workflow status parser fails closed for invalid envelopes and reasons", () => {
  const valid = workflowStatusEvent({ reason: "Initial classification." });
  const invalid = [
    workflowStatusEvent({ pubkey: ATTACKER, id: "b".repeat(64) }),
    workflowStatusEvent({
      id: "c".repeat(64),
      state: "ready-for-test",
    }),
    workflowStatusEvent({
      id: "d".repeat(64),
      reason: "  whitespace is not allowed  ",
    }),
    workflowStatusEvent({
      id: "e".repeat(64),
      tags: [["unknown", "tag"]],
    }),
    workflowStatusEvent({
      id: "f".repeat(64),
      tags: [["state", "backlog"]],
    }),
  ];

  const issue = eventToProjectIssue(issueEvent(), [], [valid, ...invalid]);

  assert.equal(issue.status, "Triage");
  assert.equal(issue.workflowStatus?.eventId, valid.id);
  assert.equal(
    issue.comments.some((comment) => comment.id === valid.id),
    false,
  );
  assert.equal(
    issue.comments.some((comment) => comment.id === invalid[0].id),
    true,
  );
});

test("malformed workflow-status lookalikes remain ordinary discussion", () => {
  const malformed = workflowStatusEvent({
    id: "b".repeat(64),
    state: "ready-for-test",
  });
  const issue = eventToProjectIssue(issueEvent(), [], [malformed]);

  assert.equal(issue.status, "Triage");
  assert.equal(issue.activity.length, 1);
  assert.equal(issue.comments[0]?.id, malformed.id);
});

test("a valid human accepted verdict remains the only Done authority", () => {
  const workflowDoneAdjacent = workflowStatusEvent({
    state: "ready-for-test",
    reason: "Ready for human testing.",
    createdAt: 400,
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [acceptedReviewVerdict()],
    [currentReviewMarker(), workflowDoneAdjacent],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
  assert.equal(issue.workflowStatus?.state, "ready-for-test");
});

test("custom workflow status ignores malformed envelope variants", () => {
  const valid = workflowStatusEvent({ reason: "Initial classification." });
  const invalid = [
    workflowStatusEvent({ id: "b".repeat(64), kind: 2 }),
    workflowStatusEvent({ id: "c".repeat(64), tags: [["a", "wrong"]] }),
    workflowStatusEvent({
      id: "d".repeat(64),
      tags: [["e", "e".repeat(64), "", "reply"]],
    }),
    workflowStatusEvent({ id: "f".repeat(64), tags: [["workflow", "wrong"]] }),
    workflowStatusEvent({ id: "1".repeat(64), state: "unknown" }),
    workflowStatusEvent({ id: "2".repeat(64), reason: "line\nbreak" }),
  ];
  const issue = eventToProjectIssue(issueEvent(), [], [valid, ...invalid]);

  assert.equal(issue.status, "Triage");
  assert.equal(issue.workflowStatus?.eventId, valid.id);
  assert.equal(issue.activity.length, 1);
});

test("same-second custom statuses use the greatest event id and survive reload", () => {
  const first = workflowStatusEvent({
    id: "a".repeat(64),
    reason: "Initial classification.",
    createdAt: 100,
  });
  const second = workflowStatusEvent({
    id: "b".repeat(64),
    state: "implemented",
    createdAt: 100,
  });
  const firstRead = eventToProjectIssue(
    issueEvent(),
    [],
    [first, second, first],
  );
  const reloadRead = eventToProjectIssue(issueEvent(), [], [second, first]);

  assert.equal(firstRead.status, "Implemented");
  assert.equal(reloadRead.workflowStatus?.eventId, second.id);
  assert.equal(reloadRead.status, firstRead.status);
});

test("frozen mybuzz-status-v1 write, reload, and read fixture is stable", () => {
  const first = workflowStatusEvent({
    id: "a".repeat(64),
    createdAt: 100,
    reason: "Initial classification.",
  });
  const latest = workflowStatusEvent({
    id: "b".repeat(64),
    createdAt: 100,
    state: "implemented",
  });
  const written = eventToProjectIssue(issueEvent(), [], [first, latest]);
  const reloaded = eventToProjectIssue(issueEvent(), [], [latest, first]);

  assert.equal(written.status, "Implemented");
  assert.equal(written.workflowStatus?.eventId, latest.id);
  assert.equal(reloaded.status, written.status);
  assert.equal(reloaded.workflowStatus?.eventId, latest.id);
});

test("native and malformed status events have no header or activity effect", () => {
  const valid = workflowStatusEvent({ reason: "Initial classification." });
  const malformed = [
    workflowStatusEvent({ id: "b".repeat(64), kind: 1631 }),
    workflowStatusEvent({ id: "c".repeat(64), tags: [["state", "backlog"]] }),
    workflowStatusEvent({ id: "d".repeat(64), tags: [["unknown", "tag"]] }),
    workflowStatusEvent({ id: "f".repeat(64), reason: "bad\u0000reason" }),
  ];
  const nativeResolved = statusEvent({
    kind: 1631,
    pubkey: OWNER,
    createdAt: 999,
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [nativeResolved],
    [valid, ...malformed],
  );

  assert.equal(issue.status, "Triage");
  assert.equal(issue.activity.length, 1);
  assert.equal(issue.activity[0]?.id, valid.id);
  assert.deepEqual(
    issue.comments.map((comment) => comment.id).sort(),
    malformed.map((event) => event.id).sort(),
  );
});

test("activity keeps trusted assignment and technical milestones separate from discussion", () => {
  const assignment = assignmentComment(OWNER, [ATTACKER], "a".repeat(64));
  const milestone = {
    ...workflowStatusEvent({ id: "b".repeat(64) }),
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "test-ready"],
    ],
  };
  const issue = eventToProjectIssue(issueEvent(), [], [assignment, milestone]);

  assert.equal(
    issue.activity.some((entry) => entry.text === "Assigned: cccccccccccc..."),
    true,
  );
  assert.equal(
    issue.activity.some(
      (entry) => entry.text === "Technical milestone: test-ready",
    ),
    true,
  );
  assert.equal(
    issue.comments.some((comment) => comment.id === milestone.id),
    false,
  );
});

function issueEvent(overrides = {}) {
  return {
    id: "e".repeat(64),
    kind: 1621,
    pubkey: AUTHOR,
    created_at: 100,
    content: "Something is broken",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Something is broken"],
    ],
    ...overrides,
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
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

function assignmentComment(
  pubkey,
  assignees,
  id,
  label = ISSUE_ASSIGNMENT_LABEL,
  createdAt = 200,
  prior,
) {
  return {
    id,
    kind: 1,
    pubkey,
    created_at: createdAt,
    content:
      label === ISSUE_ASSIGNMENT_LABEL
        ? "Assigned this issue"
        : "Unassigned this issue",
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ...assignees.map((value) => ["p", value]),
      ["t", label],
      ...(prior ? [["prior", prior]] : []),
    ],
  };
}

function currentReviewMarker() {
  return {
    id: "3".repeat(64),
    kind: 1,
    pubkey: REVIEW_COORDINATOR,
    created_at: 250,
    content: `[REVIEW-READY]\nReview-ID: ${REVIEW_ID}\nTarget: immutable-target\nEvidence: focused tests\nTest: open the issue\nKnown limitations: none`,
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "review-ready"],
      ["review", REVIEW_ID],
      ["p", OWNER],
      ["p", REVIEW_TESTER],
    ],
  };
}

function acceptedReviewVerdict(overrides = {}) {
  return {
    id: "4".repeat(64),
    kind: 1631,
    pubkey: REVIEW_TESTER,
    created_at: 300,
    content: "",
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["p", OWNER],
      ["p", AUTHOR],
      ["t", "human-verdict"],
      ["verdict", "accepted"],
      ["review", REVIEW_ID],
    ],
    ...overrides,
  };
}

function rejectedReviewVerdict(overrides = {}) {
  return {
    id: "5".repeat(64),
    kind: 1630,
    pubkey: OWNER,
    created_at: 300,
    content: "The target does not load.",
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["p", OWNER],
      ["p", AUTHOR],
      ["t", "human-verdict"],
      ["verdict", "rejected"],
      ["review", REVIEW_ID],
    ],
    ...overrides,
  };
}

function reviewConfirmation(rawVerdict, overrides = {}) {
  const verdict = rawVerdict.tags.find((tag) => tag[0] === "verdict")[1];
  const kanbanStatus = verdict === "accepted" ? "done" : "ready";
  const reason =
    verdict === "rejected" ? `\nReason: ${rawVerdict.content}` : "";
  return {
    id: "6".repeat(64),
    kind: 1,
    pubkey: REVIEW_COORDINATOR,
    created_at: 350,
    content: `[ISSUE-VERDICT-CONFIRMED]\nIssue: ${"e".repeat(64)}\nRepository: ${REPO_ADDRESS}\nReview: ${REVIEW_ID}\nVerdict-Event: ${rawVerdict.id}\nActor: ${rawVerdict.pubkey}\nVerdict: ${verdict}\nKanban-Status: ${kanbanStatus}${reason}`,
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "issue-verdict-confirmed"],
      ["review", REVIEW_ID],
      ["verdict", verdict],
      ["verdict-event", rawVerdict.id],
      ["kanban-status", kanbanStatus],
      ["p", rawVerdict.pubkey],
    ],
    ...overrides,
  };
}

const REVIEW_AUTHORITY = {
  coordinatorPubkeys: [REVIEW_COORDINATOR],
  humanPubkeys: [OWNER, REVIEW_TESTER],
};

test("native NIP-34 status events cannot change the custom workflow header", () => {
  const attackerClosed = statusEvent({
    kind: 1632,
    pubkey: ATTACKER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(issueEvent(), [attackerClosed]);

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.TRIAGE);
});

test.skip("honors status events from the issue author and repo owner", () => {
  const authorDone = statusEvent({
    kind: 1631,
    pubkey: AUTHOR,
    createdAt: 300,
  });
  assert.equal(
    eventToProjectIssue(issueEvent(), [authorDone]).status,
    PROJECT_ISSUE_STATUS.DONE,
  );

  const ownerClosed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 300,
  });
  assert.equal(
    eventToProjectIssue(issueEvent(), [ownerClosed]).status,
    PROJECT_ISSUE_STATUS.CLOSED,
  );
});

test.skip("honors a status event from the verified NIP-OA owner only when explicitly supplied", () => {
  const oaOwnerDone = statusEvent({
    kind: 1631,
    pubkey: OA_OWNER,
    createdAt: 300,
  });

  assert.equal(
    eventToProjectIssue(issueEvent(), [oaOwnerDone]).status,
    PROJECT_ISSUE_STATUS.BACKLOG,
  );
  assert.equal(
    eventToProjectIssue(issueEvent(), [oaOwnerDone], [], [OA_OWNER]).status,
    PROJECT_ISSUE_STATUS.DONE,
  );
});

test.skip("honors status events from an assignee", () => {
  const assignee = "d".repeat(64);
  const assignment = assignmentComment(AUTHOR, [assignee], "assign-1");
  const assigneeDone = statusEvent({
    kind: 1631,
    pubkey: assignee,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(issueEvent(), [assigneeDone], [assignment]);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
  assert.deepEqual(issue.assignees, [assignee]);
});

test.skip("still ignores status events from a non-assignee stranger", () => {
  const assignee = "d".repeat(64);
  const assignment = assignmentComment(AUTHOR, [assignee], "assign-1");
  const strangerDone = statusEvent({
    kind: 1631,
    pubkey: ATTACKER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(issueEvent(), [strangerDone], [assignment]);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
});

test.skip("exposes the timestamp of the status event it honored", () => {
  const ownerClosed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 300,
  });
  const issue = eventToProjectIssue(issueEvent(), [ownerClosed]);

  assert.equal(issue.statusCreatedAt, 300);
  assert.equal(eventToProjectIssue(issueEvent()).statusCreatedAt, null);
});

test.skip("a follow-up status change outranks the one it replaces", () => {
  const ownerClosed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 300,
  });
  const issue = eventToProjectIssue(issueEvent(), [ownerClosed]);

  // Nostr timestamps are whole seconds: reopening within the same second must
  // still sort above the close it replaces.
  assert.equal(nextProjectIssueStatusCreatedAt(issue, 300), 301);
  assert.equal(nextProjectIssueStatusCreatedAt(issue, 900), 900);
  assert.equal(
    nextProjectIssueStatusCreatedAt(eventToProjectIssue(issueEvent()), 900),
    900,
  );
});

test.skip("tag helpers drop malformed value-less tags", () => {
  const event = issueEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["t"],
      ["t", ""],
      ["t", "bug"],
      ["p"],
      ["subject"],
    ],
  });

  assert.deepEqual(getAllTags(event, "t"), ["bug"]);
  assert.deepEqual(getAllTags(event, "p"), []);
  assert.equal(getTag(event, "subject"), undefined);

  const issue = eventToProjectIssue(event);
  assert.deepEqual(issue.labels, ["bug"]);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
  assert.equal(issue.title, "Something is broken");
});

test("preserves root and comment tags for rich content rendering", () => {
  const root = issueEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Something is broken"],
      ["imeta", "url https://relay.example/media/root.png", "m image/png"],
    ],
  });
  const comment = {
    id: "comment-rich-content",
    kind: 1,
    pubkey: ATTACKER,
    created_at: 200,
    content: "![Screenshot](https://relay.example/media/comment.png)",
    tags: [
      ["e", root.id, "", "root"],
      ["imeta", "url https://relay.example/media/comment.png", "m image/png"],
    ],
  };

  const issue = eventToProjectIssue(root, [], [comment]);

  assert.deepEqual(issue.tags, [root.tags[2]]);
  assert.deepEqual(issue.comments[0].tags, [comment.tags[1]]);
});

test.skip("does not infer a review from status-event prose", () => {
  const root = issueEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Something is broken"],
      ["t", "approved"],
    ],
  });
  const reviewReady = {
    ...statusEvent({ kind: 1630, pubkey: OWNER, createdAt: 150 }),
    content: "## LIVE / Review-ready\n\nKanban KBN-t_example stands on Review.",
  };

  assert.equal(
    eventToProjectIssue(root, [reviewReady]).status,
    PROJECT_ISSUE_STATUS.APPROVED,
  );
});

test.skip("a marker-only hexadecimal review binding yields In Review without a card", () => {
  const TESTER = "f".repeat(64);
  const COORDINATOR = "1".repeat(64);
  const marker = {
    id: "3".repeat(64),
    kind: 1,
    pubkey: COORDINATOR,
    created_at: 250,
    content: `[REVIEW-READY]\nReview-ID: ${REVIEW_ID}\nTarget: immutable-target\nEvidence: focused tests\nTest: open the issue\nKnown limitations: none`,
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "review-ready"],
      ["review", REVIEW_ID],
      ["p", OWNER],
      ["p", TESTER],
    ],
  };

  const issue = eventToProjectIssue(issueEvent(), [], [marker], [], {
    coordinatorPubkeys: [COORDINATOR],
    humanPubkeys: [OWNER, TESTER],
  });

  assert.deepEqual(issue.currentReview, {
    id: REVIEW_ID,
    target: "immutable-target",
    evidence: "focused tests",
    test: "open the issue",
    limitations: "none",
    authorizedHumanPubkeys: [OWNER, TESTER],
    verdict: null,
  });
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
});

test("a direct human accepted verdict bound to the current review is authoritative", () => {
  const accepted = acceptedReviewVerdict();
  const issue = eventToProjectIssue(
    issueEvent(),
    [accepted],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
  assert.equal(issue.currentReview.verdict.kind, "accepted");
  assert.equal(issue.currentReview.verdict.confirmation, null);
  assert.equal(issue.statusEventId, accepted.id);
});

test("an exact trusted confirmation remains audit metadata after direct Done", () => {
  const accepted = acceptedReviewVerdict();
  const confirmation = reviewConfirmation(accepted);
  const issue = eventToProjectIssue(
    issueEvent(),
    [accepted],
    [currentReviewMarker(), confirmation],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
  assert.equal(issue.statusEventId, accepted.id);
  assert.equal(issue.statusCreatedAt, accepted.created_at);
  assert.equal(issue.currentReview.verdict.confirmation.kanbanStatus, "done");
});

test.skip("an exact trusted rejected confirmation returns the issue to Backlog", () => {
  const rejected = rejectedReviewVerdict();
  const pending = eventToProjectIssue(
    issueEvent(),
    [rejected],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );
  assert.equal(pending.status, PROJECT_ISSUE_STATUS.BACKLOG);
  assert.equal(pending.currentReview.verdict.kind, "rejected");
  assert.equal(pending.currentReview.verdict.confirmation, null);

  for (const kanbanStatus of ["ready", "todo"]) {
    const base = reviewConfirmation(rejected);
    const confirmation = {
      ...base,
      content: base.content.replace(
        "Kanban-Status: ready",
        `Kanban-Status: ${kanbanStatus}`,
      ),
      tags: base.tags.map((tag) =>
        tag[0] === "kanban-status" ? ["kanban-status", kanbanStatus] : tag,
      ),
    };
    const issue = eventToProjectIssue(
      issueEvent(),
      [rejected],
      [currentReviewMarker(), confirmation],
      [],
      REVIEW_AUTHORITY,
    );
    assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
    assert.equal(
      issue.currentReview.verdict.confirmation.kanbanStatus,
      kanbanStatus,
    );
  }
});

test.skip("a pending current review suppresses a generic resolved status", () => {
  const genericDone = statusEvent({
    kind: 1631,
    pubkey: OWNER,
    createdAt: 300,
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [genericDone],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
});

test.skip("ordinary lifecycle authority remains effective after the current marker", () => {
  const closed = statusEvent({
    kind: 1632,
    pubkey: OWNER,
    createdAt: 300,
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [closed],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.CLOSED);
});

test.skip("raw verdicts require exact recipients and accepted content", () => {
  const accepted = acceptedReviewVerdict();
  const malformedVerdicts = [
    { ...accepted, content: "accepted" },
    {
      ...accepted,
      tags: accepted.tags.filter((tag) => tag[0] !== "p"),
    },
    {
      ...accepted,
      tags: [...accepted.tags, ["p", OWNER]],
    },
  ];

  for (const verdict of malformedVerdicts) {
    const issue = eventToProjectIssue(
      issueEvent(),
      [verdict],
      [currentReviewMarker()],
      [],
      REVIEW_AUTHORITY,
    );
    assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
    assert.equal(issue.currentReview.verdict, null);
  }
});

test("malformed confirmations cannot override direct Done", () => {
  const accepted = acceptedReviewVerdict();
  const valid = reviewConfirmation(accepted);
  const cases = [
    [
      {
        ...valid,
        pubkey: ATTACKER,
      },
    ],
    [
      {
        ...valid,
        created_at: accepted.created_at - 1,
      },
    ],
    [
      {
        ...valid,
        tags: [...valid.tags, ["review", REVIEW_ID]],
      },
    ],
    [
      {
        ...valid,
        content: valid.content.replace("Actor: ", "Actor: wrong-"),
      },
    ],
    [valid, { ...valid, id: "7".repeat(64) }],
    [
      valid,
      {
        ...valid,
        id: "8".repeat(64),
        tags: [...valid.tags, ["review", REVIEW_ID]],
      },
    ],
  ];

  for (const confirmations of cases) {
    const issue = eventToProjectIssue(
      issueEvent(),
      [accepted],
      [currentReviewMarker(), ...confirmations],
      [],
      REVIEW_AUTHORITY,
    );
    assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
    assert.equal(issue.currentReview.verdict.confirmation, null);
  }
});

test("a review marker for a malformed repository coordinate fails closed", () => {
  const malformedAddress = "not-a-repository-coordinate";
  const root = issueEvent({
    tags: [
      ["a", malformedAddress],
      ["subject", "Something is broken"],
    ],
  });
  const marker = {
    ...currentReviewMarker(),
    tags: currentReviewMarker().tags.map((tag) =>
      tag[0] === "a" ? ["a", malformedAddress] : tag,
    ),
  };

  assert.equal(
    eventToProjectIssue(root, [], [marker], [], REVIEW_AUTHORITY).currentReview,
    null,
  );
});

test.skip("a review marker predating the issue fails closed", () => {
  const marker = { ...currentReviewMarker(), created_at: 99 };
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [marker],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.currentReview, null);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
});

test.skip("a stale accepted verdict does not resolve the current review", () => {
  const stale = acceptedReviewVerdict({
    tags: acceptedReviewVerdict().tags.map((tag) =>
      tag[0] === "review" ? ["review", "stale-review"] : tag,
    ),
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [stale],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
});

test.skip("an untrusted accepted verdict does not resolve the current review", () => {
  const issue = eventToProjectIssue(
    issueEvent(),
    [acceptedReviewVerdict({ pubkey: ATTACKER })],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
});

test.skip("an accepted verdict predating the review marker fails closed", () => {
  const issue = eventToProjectIssue(
    issueEvent(),
    [acceptedReviewVerdict({ created_at: 249 })],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
});

test("rejects review-ready markers with duplicate review bindings", () => {
  const TESTER = "f".repeat(64);
  const COORDINATOR = "1".repeat(64);
  const marker = {
    id: "8".repeat(64),
    kind: 1,
    pubkey: COORDINATOR,
    created_at: 250,
    content:
      "[REVIEW-READY]\nReview-ID: buzz-workflow:t_1:revision\nTarget: immutable-target\nEvidence: focused tests\nTest: open the issue\nKnown limitations: none",
    tags: [
      ["e", "e".repeat(64), "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "review-ready"],
      ["review", "buzz-workflow:t_1:revision"],
      ["review", "stale-review", "unexpected"],
      ["review-root", "2".repeat(64)],
      ["p", OWNER],
      ["p", TESTER],
    ],
  };

  assert.equal(
    eventToProjectIssue(issueEvent(), [], [marker], [], {
      coordinatorPubkeys: [COORDINATOR],
      humanPubkeys: [OWNER, TESTER],
    }).currentReview,
    null,
  );

  const conflictingRoot = {
    ...marker,
    tags: [
      ...marker.tags.filter(
        (tag) => tag[0] !== "review" || tag[1] === "buzz-workflow:t_1:revision",
      ),
      ["e", "f".repeat(64), "", "root"],
    ],
  };
  assert.equal(
    eventToProjectIssue(issueEvent(), [], [conflictingRoot], [], {
      coordinatorPubkeys: [COORDINATOR],
      humanPubkeys: [OWNER, TESTER],
    }).currentReview,
    null,
  );
});

test.skip("rejects a legacy nonhex card-derived review binding", () => {
  const marker = {
    ...currentReviewMarker(),
    tags: currentReviewMarker().tags.map((tag) =>
      tag[0] === "review" ? ["review", "buzz-workflow:t_1:revision"] : tag,
    ),
  };
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [marker],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.currentReview, null);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
});

test.skip("rejects a review-ready marker carrying the retired review-root tag", () => {
  const marker = {
    ...currentReviewMarker(),
    tags: [...currentReviewMarker().tags, ["review-root", "2".repeat(64)]],
  };
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [marker],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.currentReview, null);
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
});

test.skip("rejects a human verdict carrying the retired review-root tag", () => {
  const accepted = acceptedReviewVerdict({
    tags: [...acceptedReviewVerdict().tags, ["review-root", "2".repeat(64)]],
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [accepted],
    [currentReviewMarker()],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.IN_REVIEW);
  assert.equal(issue.currentReview?.verdict, null);
});

test("treats a confirmation carrying the retired review-root tag as non-authoritative", () => {
  const accepted = acceptedReviewVerdict();
  const confirmation = reviewConfirmation(accepted, {
    tags: [
      ...reviewConfirmation(accepted).tags,
      ["review-root", "2".repeat(64)],
    ],
  });
  const issue = eventToProjectIssue(
    issueEvent(),
    [accepted],
    [currentReviewMarker(), confirmation],
    [],
    REVIEW_AUTHORITY,
  );

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.DONE);
  assert.equal(issue.currentReview?.verdict?.confirmation, null);
});

test.skip("recognizes approved status-event labels and action-required comment metadata", () => {
  const root = issueEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Something is broken"],
    ],
  });
  const approved = {
    id: "approved-status",
    kind: 1630,
    pubkey: OWNER,
    created_at: 150,
    content: "",
    tags: [
      ["e", root.id, "", "root"],
      ["a", REPO_ADDRESS],
      ["t", "approved"],
    ],
  };
  const comment = {
    id: "comment-action-required",
    kind: 1,
    pubkey: OWNER,
    created_at: 200,
    content: "Test: verify the Windows installer",
    tags: [
      ["e", root.id, "", "root"],
      ["p", AUTHOR],
      ["t", ISSUE_ACTION_REQUIRED_LABEL],
    ],
  };

  const issue = eventToProjectIssue(root, [approved], [comment]);

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.APPROVED);
  assert.equal(issue.comments[0].actionRequired, true);
  assert.deepEqual(issue.comments[0].recipients, [AUTHOR]);
});

test("human-directed issue comments require a leading action prefix", () => {
  assert.equal(
    isHumanDirectedIssueComment("Test: verify the Windows installer"),
    true,
  );
  assert.equal(
    isHumanDirectedIssueComment("Evidence:\nExpected: HTTP 200"),
    false,
  );
  assert.equal(
    isHumanDirectedIssueComment("Build passed\nReply: not requested"),
    false,
  );
});

test("parses public and private-safe issue provenance", () => {
  const channelId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  const publicIssue = eventToProjectIssue(
    issueEvent({
      tags: [
        ["a", REPO_ADDRESS],
        ["h", channelId],
      ],
    }),
  );
  const privateIssue = eventToProjectIssue(
    issueEvent({
      tags: [
        ["a", REPO_ADDRESS],
        ["buzz-origin-agent", "Builder"],
      ],
    }),
  );

  assert.equal(publicIssue.channelId, channelId);
  assert.equal(publicIssue.originAgentName, null);
  assert.equal(privateIssue.channelId, null);
  assert.equal(privateIssue.originAgentName, "Builder");
});

test("assignees follow trusted assignment operations in deterministic order", () => {
  const assignee = "d".repeat(64);
  const otherAssignee = "f".repeat(64);
  const volunteer = "5".repeat(64);

  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      // Author assigns (self-assignment included) — trusted.
      assignmentComment(AUTHOR, [assignee.toUpperCase(), AUTHOR], "assign-1"),
      // Repo owner assigns — trusted; duplicate assignee dedupes.
      assignmentComment(OWNER, [assignee, otherAssignee], "assign-2"),
      // Any member self-assigning (sole p tag is the signer) — trusted.
      assignmentComment(volunteer, [volunteer], "assign-3"),
      // Untrusted signer assigning someone else — ignored.
      assignmentComment(ATTACKER, ["a".repeat(64)], "assign-4"),
      // Untrusted signer sneaking themselves in alongside others — ignored.
      assignmentComment(ATTACKER, [ATTACKER, "b".repeat(64)], "assign-5"),
      // A volunteer may remove only themselves.
      assignmentComment(
        volunteer,
        [volunteer],
        "unassign-1",
        ISSUE_UNASSIGNMENT_LABEL,
        201,
      ),
      // An untrusted signer cannot remove somebody else.
      assignmentComment(
        ATTACKER,
        [otherAssignee],
        "unassign-2",
        ISSUE_UNASSIGNMENT_LABEL,
        202,
      ),
      // Repo owner may remove any assignee.
      assignmentComment(
        OWNER,
        [otherAssignee],
        "unassign-3",
        ISSUE_UNASSIGNMENT_LABEL,
        203,
      ),
      // Same-second operations use event id as a stable tie-breaker:
      // assign sorts before unassign here, leaving the assignee removed.
      assignmentComment(OWNER, [otherAssignee], "a-assign", undefined, 204),
      assignmentComment(
        OWNER,
        [otherAssignee],
        "z-unassign",
        ISSUE_UNASSIGNMENT_LABEL,
        204,
      ),
      // Trusted plain comment without the label adds nothing.
      {
        id: "plain-comment",
        kind: 1,
        pubkey: AUTHOR,
        created_at: 201,
        content: "Just a comment",
        tags: [
          ["e", "e".repeat(64), "", "root"],
          ["p", ATTACKER],
        ],
      },
    ],
  );

  assert.deepEqual(issue.assignees.sort(), [AUTHOR, assignee].sort());
});

test("owner unassignment overrides a future-dated self-assignment", () => {
  const volunteer = "5".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      assignmentComment(
        volunteer,
        [volunteer],
        "future-self-assign",
        undefined,
        1_000,
      ),
      assignmentComment(
        OWNER,
        [volunteer],
        "owner-unassign",
        ISSUE_UNASSIGNMENT_LABEL,
        200,
      ),
    ],
  );

  assert.deepEqual(issue.assignees, []);
});

test("owner assignment overrides a future-dated self-unassignment", () => {
  const volunteer = "5".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      assignmentComment(
        volunteer,
        [volunteer],
        "future-self-unassign",
        ISSUE_UNASSIGNMENT_LABEL,
        1_000,
      ),
      assignmentComment(OWNER, [volunteer], "owner-assign", undefined, 200),
    ],
  );

  assert.deepEqual(issue.assignees, [volunteer]);
});

test("causal self-unassignment can follow an owner assignment", () => {
  const volunteer = "5".repeat(64);
  const ownerAssignmentId = "1".repeat(64);
  const selfUnassignmentId = "2".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      assignmentComment(OWNER, [volunteer], ownerAssignmentId),
      assignmentComment(
        volunteer,
        [volunteer],
        selfUnassignmentId,
        ISSUE_UNASSIGNMENT_LABEL,
        300,
        ownerAssignmentId,
      ),
    ],
  );

  assert.deepEqual(issue.assignees, []);
  assert.equal(issue.assigneeOperationHeads[volunteer], selfUnassignmentId);
});

test("causal self-assignment can follow an owner unassignment", () => {
  const volunteer = "5".repeat(64);
  const ownerUnassignmentId = "3".repeat(64);
  const selfAssignmentId = "4".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      assignmentComment(
        OWNER,
        [volunteer],
        ownerUnassignmentId,
        ISSUE_UNASSIGNMENT_LABEL,
      ),
      assignmentComment(
        volunteer,
        [volunteer],
        selfAssignmentId,
        ISSUE_ASSIGNMENT_LABEL,
        300,
        ownerUnassignmentId,
      ),
    ],
  );

  assert.deepEqual(issue.assignees, [volunteer]);
  assert.equal(issue.assigneeOperationHeads[volunteer], selfAssignmentId);
});

test("ignores a causal self-operation with a stale prior", () => {
  const volunteer = "5".repeat(64);
  const initialAssignmentId = "6".repeat(64);
  const ownerUnassignmentId = "7".repeat(64);
  const staleSelfAssignmentId = "8".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      assignmentComment(OWNER, [volunteer], initialAssignmentId),
      assignmentComment(
        OWNER,
        [volunteer],
        ownerUnassignmentId,
        ISSUE_UNASSIGNMENT_LABEL,
        250,
      ),
      assignmentComment(
        volunteer,
        [volunteer],
        staleSelfAssignmentId,
        ISSUE_ASSIGNMENT_LABEL,
        300,
        initialAssignmentId,
      ),
    ],
  );

  assert.deepEqual(issue.assignees, []);
  assert.equal(issue.assigneeOperationHeads[volunteer], ownerUnassignmentId);
});

test("issue recipients remain notification routing, not assignments", () => {
  const recipient = "d".repeat(64);
  const otherRecipient = "f".repeat(64);
  const issue = eventToProjectIssue(
    issueEvent({
      tags: [
        ["a", REPO_ADDRESS],
        ["subject", "Something is broken"],
        // Routing tag every issue carries — not an assignment.
        ["p", OWNER],
        ["p", recipient.toUpperCase()],
        ["p", otherRecipient],
      ],
    }),
  );

  assert.deepEqual(issue.assignees, []);
});

test("builds repository-scoped issue creation tags", () => {
  assert.deepEqual(
    buildGitIssueTags({
      repoAddress: REPO_ADDRESS,
      repoOwner: OWNER,
      title: "  Fix the broken workflow  ",
    }),
    [
      ["a", REPO_ADDRESS],
      ["p", OWNER],
      ["subject", "Fix the broken workflow"],
    ],
  );
});

test("orders consecutive issue comments across whole-second timestamps", () => {
  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [
      {
        id: "comment-1",
        kind: 1,
        pubkey: AUTHOR,
        created_at: 200,
        content: "First",
        tags: [["e", "e".repeat(64), "", "root"]],
      },
      {
        id: "comment-2",
        kind: 1,
        pubkey: AUTHOR,
        created_at: 201,
        content: "Second",
        tags: [["e", "e".repeat(64), "", "root"]],
      },
      {
        id: "attacker-comment",
        kind: 1,
        pubkey: ATTACKER,
        created_at: 10_000,
        content: "Future",
        tags: [["e", "e".repeat(64), "", "root"]],
      },
    ],
  );

  assert.equal(nextProjectIssueCommentCreatedAt(issue, 200, AUTHOR), 202);
  assert.equal(nextProjectIssueCommentCreatedAt(issue, 300, AUTHOR), 300);
});
