import assert from "node:assert/strict";
import test from "node:test";

import {
  buildGitIssueTags,
  eventToProjectIssue,
  getAllTags,
  getTag,
  nextProjectIssueCommentCreatedAt,
  PROJECT_ISSUE_STATUS,
} from "./projectIssues.mjs";

const OWNER = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const ATTACKER = "c".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:demo`;

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

function assigneeEvent({
  pubkey,
  assignee,
  createdAt,
  id = `${createdAt.toString(16).padStart(64, "0")}`,
  tags,
}) {
  return {
    id,
    kind: 32001,
    pubkey,
    created_at: createdAt,
    content: "",
    tags: tags ?? [
      ["d", "e".repeat(64)],
      ["e", "e".repeat(64), "", "root"],
      ...(assignee === null
        ? [["assignee", "none"]]
        : [["p", assignee, "", "assignee"]]),
      ["a", REPO_ADDRESS],
    ],
  };
}

test("ignores assignment events from a different pubkey", () => {
  const attackerAssignment = assigneeEvent({
    pubkey: ATTACKER,
    assignee: ATTACKER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(issueEvent(), [], [], [attackerAssignment]);

  assert.equal(issue.assignee, null);
  assert.equal(issue.assigneeEventId, null);
});

test("only the repo owner can set shared issue routing", () => {
  const authorAssignsSelf = assigneeEvent({
    pubkey: AUTHOR,
    assignee: AUTHOR,
    createdAt: 400,
  });
  const ownerReassigns = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [],
    [authorAssignsSelf, ownerReassigns],
  );

  assert.equal(issue.assignee, OWNER);
  assert.equal(issue.assigneeEventId, ownerReassigns.id);
  assert.equal(issue.assignedBy, OWNER);
});

test("ignores ambiguous assignment envelopes", () => {
  const assignment = assigneeEvent({
    pubkey: OWNER,
    assignee: AUTHOR,
    createdAt: 200,
  });
  const ambiguous = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 300,
    tags: [
      ["d", "e".repeat(64)],
      ["e", "e".repeat(64), "", "root"],
      ["p", OWNER, "", "assignee"],
      ["p", ATTACKER],
      ["a", REPO_ADDRESS],
    ],
  });
  const duplicateRoot = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 400,
    tags: [
      ["d", "e".repeat(64)],
      ["e", "e".repeat(64), "", "root"],
      ["e", "e".repeat(64), "", "root"],
      ["p", OWNER, "", "assignee"],
      ["a", REPO_ADDRESS],
    ],
  });
  const strayChannel = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 500,
    tags: [
      ["d", "e".repeat(64)],
      ["e", "e".repeat(64), "", "root"],
      ["p", OWNER, "", "assignee"],
      ["a", REPO_ADDRESS],
      ["h", "00000000-0000-0000-0000-000000000000"],
    ],
  });
  const wrongReplacementKey = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 600,
    tags: [
      ["d", "f".repeat(64)],
      ["e", "e".repeat(64), "", "root"],
      ["p", OWNER, "", "assignee"],
      ["a", REPO_ADDRESS],
    ],
  });

  const issue = eventToProjectIssue(
    issueEvent(),
    [],
    [],
    [assignment, ambiguous, duplicateRoot, strayChannel, wrongReplacementKey],
  );
  assert.equal(issue.assignee, AUTHOR);
  assert.equal(issue.assigneeEventId, assignment.id);
});

test("explicit unassignment wins without treating malformed events as unassignment", () => {
  const assignment = assigneeEvent({
    pubkey: OWNER,
    assignee: AUTHOR,
    createdAt: 200,
  });
  const malformed = assigneeEvent({
    pubkey: OWNER,
    assignee: "not-a-pubkey",
    createdAt: 300,
  });
  const stillAssigned = eventToProjectIssue(
    issueEvent(),
    [],
    [],
    [assignment, malformed],
  );
  assert.equal(stillAssigned.assignee, AUTHOR);
  assert.equal(stillAssigned.assigneeEventId, assignment.id);

  const unassignment = assigneeEvent({
    pubkey: OWNER,
    assignee: null,
    createdAt: 400,
  });
  const unassigned = eventToProjectIssue(
    issueEvent(),
    [],
    [],
    [assignment, malformed, unassignment],
  );
  assert.equal(unassigned.assignee, null);
  assert.equal(unassigned.assigneeEventId, unassignment.id);
  assert.equal(unassigned.assignedBy, OWNER);
});

test("same-second assignments use the lowest event id as a deterministic tie-break", () => {
  const lowerId = assigneeEvent({
    pubkey: OWNER,
    assignee: AUTHOR,
    createdAt: 300,
    id: "1".repeat(64),
  });
  const higherId = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 300,
    id: "f".repeat(64),
  });

  for (const events of [
    [lowerId, higherId],
    [higherId, lowerId],
  ]) {
    const issue = eventToProjectIssue(issueEvent(), [], [], events);
    assert.equal(issue.assignee, AUTHOR);
    assert.equal(issue.assigneeEventId, lowerId.id);
  }
});

test("ignores status events from a different pubkey", () => {
  const attackerClosed = statusEvent({
    kind: 1632,
    pubkey: ATTACKER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(issueEvent(), [attackerClosed]);

  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
});

test("honors status events from the issue author and repo owner", () => {
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

test("tag helpers drop malformed value-less tags", () => {
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
