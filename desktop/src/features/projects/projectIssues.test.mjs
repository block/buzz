import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";

import {
  buildGitIssueTags,
  eventToProjectIssue,
  getAllTags,
  getTag,
  nextProjectIssueCommentCreatedAt,
  PROJECT_ISSUE_STATUS,
} from "./projectIssues.mjs";

const OWNER_SECRET = new Uint8Array(32).fill(1);
const AUTHOR_SECRET = new Uint8Array(32).fill(2);
const ATTACKER_SECRET = new Uint8Array(32).fill(3);
const OWNER = getPublicKey(OWNER_SECRET);
const AUTHOR = getPublicKey(AUTHOR_SECRET);
const ATTACKER = getPublicKey(ATTACKER_SECRET);
const REPO_ADDRESS = `30617:${OWNER}:demo`;

function secretForPubkey(pubkey) {
  if (pubkey === OWNER) return OWNER_SECRET;
  if (pubkey === AUTHOR) return AUTHOR_SECRET;
  if (pubkey === ATTACKER) return ATTACKER_SECRET;
  throw new Error(`No test secret for ${pubkey}`);
}

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

function signedIssueEvent(overrides = {}) {
  return finalizeEvent(
    {
      kind: 1621,
      created_at: 100,
      content: "Something is broken",
      tags: [
        ["a", REPO_ADDRESS],
        ["subject", "Something is broken"],
      ],
      ...overrides,
    },
    AUTHOR_SECRET,
  );
}

const SIGNED_ISSUE_ID = signedIssueEvent().id;

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

function assigneeEvent({ pubkey, assignee, createdAt, tags }) {
  return finalizeEvent(
    {
      kind: 32001,
      created_at: createdAt,
      content: "",
      tags: tags ?? [
        ["d", SIGNED_ISSUE_ID],
        ["e", SIGNED_ISSUE_ID, "", "root"],
        ...(assignee === null
          ? [["assignee", "none"]]
          : [["p", assignee, "", "assignee"]]),
        ["a", REPO_ADDRESS],
      ],
    },
    secretForPubkey(pubkey),
  );
}

test("ignores assignment events from a different pubkey", () => {
  const attackerAssignment = assigneeEvent({
    pubkey: ATTACKER,
    assignee: ATTACKER,
    createdAt: 300,
  });

  const issue = eventToProjectIssue(
    signedIssueEvent(),
    [],
    [],
    [attackerAssignment],
  );

  assert.equal(issue.assignee, null);
  assert.equal(issue.assigneeEventId, null);
});

test("issue author and repo owner share latest-write-wins routing authority", () => {
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
    signedIssueEvent(),
    [],
    [],
    [authorAssignsSelf, ownerReassigns],
  );

  assert.equal(issue.assignee, AUTHOR);
  assert.equal(issue.assigneeEventId, authorAssignsSelf.id);
  assert.equal(issue.assignedBy, AUTHOR);

  const ownerUpdatesLater = assigneeEvent({
    pubkey: OWNER,
    assignee: ATTACKER,
    createdAt: 500,
  });
  const reassigned = eventToProjectIssue(
    signedIssueEvent(),
    [],
    [],
    [authorAssignsSelf, ownerReassigns, ownerUpdatesLater],
  );
  assert.equal(reassigned.assignee, ATTACKER);
  assert.equal(reassigned.assigneeEventId, ownerUpdatesLater.id);
  assert.equal(reassigned.assignedBy, OWNER);
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
      ["d", SIGNED_ISSUE_ID],
      ["e", SIGNED_ISSUE_ID, "", "root"],
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
      ["d", SIGNED_ISSUE_ID],
      ["e", SIGNED_ISSUE_ID, "", "root"],
      ["e", SIGNED_ISSUE_ID, "", "root"],
      ["p", OWNER, "", "assignee"],
      ["a", REPO_ADDRESS],
    ],
  });
  const strayChannel = assigneeEvent({
    pubkey: OWNER,
    assignee: OWNER,
    createdAt: 500,
    tags: [
      ["d", SIGNED_ISSUE_ID],
      ["e", SIGNED_ISSUE_ID, "", "root"],
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
      ["e", SIGNED_ISSUE_ID, "", "root"],
      ["p", OWNER, "", "assignee"],
      ["a", REPO_ADDRESS],
    ],
  });

  const issue = eventToProjectIssue(
    signedIssueEvent(),
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
    signedIssueEvent(),
    [],
    [],
    [assignment, malformed],
  );
  assert.equal(stillAssigned.assignee, AUTHOR);
  assert.equal(stillAssigned.assigneeEventId, assignment.id);

  const unassignment = assigneeEvent({
    pubkey: AUTHOR,
    assignee: null,
    createdAt: 400,
  });
  const unassigned = eventToProjectIssue(
    signedIssueEvent(),
    [],
    [],
    [assignment, malformed, unassignment],
  );
  assert.equal(unassigned.assignee, null);
  assert.equal(unassigned.assigneeEventId, unassignment.id);
  assert.equal(unassigned.assignedBy, AUTHOR);
});

test("same-second cross-author assignments use the lowest event id as a deterministic tie-break", () => {
  const lowerId = assigneeEvent({
    pubkey: OWNER,
    assignee: AUTHOR,
    createdAt: 300,
  });
  const higherId = assigneeEvent({
    pubkey: AUTHOR,
    assignee: OWNER,
    createdAt: 300,
  });
  const expected = [lowerId, higherId].sort((left, right) =>
    left.id.localeCompare(right.id),
  )[0];

  for (const events of [
    [lowerId, higherId],
    [higherId, lowerId],
  ]) {
    const issue = eventToProjectIssue(signedIssueEvent(), [], [], events);
    assert.equal(issue.assignee, expected.pubkey === OWNER ? AUTHOR : OWNER);
    assert.equal(issue.assigneeEventId, expected.id);
  }
});

test("assignment projection requires signed canonical roots and heads", () => {
  const assignment = assigneeEvent({
    pubkey: OWNER,
    assignee: AUTHOR,
    createdAt: 200,
  });
  const tamperedAssignment = {
    ...JSON.parse(JSON.stringify(assignment)),
    created_at: 300,
  };
  const duplicateRepoRoot = signedIssueEvent({
    tags: [
      ["a", REPO_ADDRESS],
      ["a", REPO_ADDRESS],
      ["subject", "Something is broken"],
    ],
  });
  const tamperedRoot = {
    ...JSON.parse(JSON.stringify(signedIssueEvent())),
    content: "tampered",
  };

  assert.equal(
    eventToProjectIssue(signedIssueEvent(), [], [], [tamperedAssignment])
      .assignee,
    null,
  );
  assert.equal(
    eventToProjectIssue(duplicateRepoRoot, [], [], [assignment]).assignee,
    null,
  );
  assert.equal(
    eventToProjectIssue(tamperedRoot, [], [], [assignment]).assignee,
    null,
  );
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
