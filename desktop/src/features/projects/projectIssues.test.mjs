import assert from "node:assert/strict";
import test from "node:test";

import {
  buildGitIssueTags,
  eventToProjectIssue,
  getAllTags,
  getTag,
  ISSUE_ASSIGNMENT_LABEL,
  ISSUE_UNASSIGNMENT_LABEL,
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
  assert.equal(issue.category, "issue");
  assert.equal(issue.status, PROJECT_ISSUE_STATUS.BACKLOG);
  assert.equal(issue.title, "Something is broken");
});

test("derives task categories from labels while defaulting legacy tasks to issue", () => {
  const changeRequest = eventToProjectIssue(
    issueEvent({
      tags: [
        ["a", REPO_ADDRESS],
        ["subject", "Update the release workflow"],
        ["t", "change-request"],
        ["t", "release"],
      ],
    }),
  );

  assert.equal(changeRequest.category, "change-request");
  assert.deepEqual(changeRequest.labels, ["change-request", "release"]);
  assert.equal(eventToProjectIssue(issueEvent()).category, "issue");
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
