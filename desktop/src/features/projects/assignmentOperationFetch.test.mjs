import assert from "node:assert/strict";
import test from "node:test";

import {
  fetchAssignmentOperationEvents,
  mergeEventsById,
} from "./assignmentOperationFetch.ts";
import { fetchProjectsWorkItems } from "./projectWorkItems.ts";

const REPO_OWNER = "a".repeat(64);
const REPO_ADDRESS = `30617:${REPO_OWNER}:relay`;
const ISSUE_ID = "1".repeat(64);
const ASSIGNEE = "b".repeat(64);

function makeIssue() {
  return {
    id: ISSUE_ID,
    kind: 1621,
    pubkey: REPO_OWNER,
    created_at: 100,
    content: "An issue",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Fix the thing"],
    ],
  };
}

/** Owner-signed assignment operation (kind:1, `t: assignment`). */
function makeAssignment(id, createdAt) {
  return {
    id,
    kind: 1,
    pubkey: REPO_OWNER,
    created_at: createdAt,
    content: "Assigned",
    tags: [
      ["e", ISSUE_ID, "", "root"],
      ["a", REPO_ADDRESS],
      ["p", ASSIGNEE],
      ["t", "assignment"],
    ],
  };
}

function makeComment(id, createdAt) {
  return {
    id,
    kind: 1,
    pubkey: REPO_OWNER,
    created_at: createdAt,
    content: `Comment ${id.slice(0, 4)}`,
    tags: [
      ["e", ISSUE_ID, "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

function eventId(index) {
  return index.toString(16).padStart(64, "0");
}

// ── fetchAssignmentOperationEvents: pagination to exhaustion ────────────────

test("fetchAssignmentOperationEvents paginates past the page limit", async () => {
  // 1,203 operations — three pages at the 500-event page limit. The stub
  // honors `until` (inclusive) and `limit` like a relay: newest first.
  const operations = Array.from({ length: 1_203 }, (_, index) =>
    makeAssignment(eventId(index + 1), 1_000_000 + index),
  );
  const calls = [];
  const fetchEvents = async (filter) => {
    calls.push(filter);
    assert.deepEqual(filter["#t"], ["assignment", "unassignment"]);
    const until = filter.until ?? Number.POSITIVE_INFINITY;
    return operations
      .filter((event) => event.created_at <= until)
      .sort((left, right) => right.created_at - left.created_at)
      .slice(0, filter.limit);
  };

  const events = await fetchAssignmentOperationEvents(
    [REPO_ADDRESS],
    fetchEvents,
  );

  assert.equal(events.length, 1_203, "every operation must be loaded");
  assert.ok(calls.length >= 3, "must page past the 500-event window");
});

test("fetchAssignmentOperationEvents stops when a page adds nothing unseen", async () => {
  // A degenerate relay that always returns the same full page must not loop
  // forever: a page with zero unseen events terminates pagination.
  const page = Array.from({ length: 500 }, (_, index) =>
    makeAssignment(eventId(index + 1), 1_000),
  );
  let callCount = 0;
  const fetchEvents = async () => {
    callCount += 1;
    return page;
  };

  const events = await fetchAssignmentOperationEvents(
    [REPO_ADDRESS],
    fetchEvents,
  );

  assert.equal(events.length, 500);
  assert.equal(callCount, 2, "second identical page must terminate the loop");
});

test("fetchAssignmentOperationEvents skips the relay for zero repositories", async () => {
  const events = await fetchAssignmentOperationEvents([], async () => {
    throw new Error("must not query the relay");
  });
  assert.deepEqual(events, []);
});

test("mergeEventsById drops duplicates and keeps both sources", () => {
  const shared = makeAssignment(eventId(1), 10);
  const merged = mergeEventsById(
    [shared, makeComment(eventId(2), 11)],
    [shared, makeAssignment(eventId(3), 12)],
  );
  assert.deepEqual(
    merged.map((event) => event.id),
    [eventId(1), eventId(2), eventId(3)],
  );
});

// ── Regression: assignment predating a full comment window ─────────────────
//
// The reduction in projectIssues.mjs is only as complete as the events it is
// handed. The general comment fetch is bounded (2,000 shared across repos in
// fetchProjectsWorkItems), so an old assignment operation can be evicted by
// newer unrelated comments. The dedicated `#t`-filtered exhaustive query must
// restore it.

test("an assignment older than 500+ newer comments still reduces to an assignee", async () => {
  const issue = makeIssue();
  // Operation at t=200, then 600 newer comments that fill the bounded window.
  const assignment = makeAssignment(eventId(9_999), 200);
  const comments = Array.from({ length: 600 }, (_, index) =>
    makeComment(eventId(index + 1), 300 + index),
  );

  const fetchEvents = async (filter) => {
    if (filter.kinds?.includes(1621)) return [issue];
    if (filter["#t"]) return [assignment];
    if (filter.kinds?.includes(1)) {
      // Bounded comment window: newest first, capped — the assignment
      // operation has been pushed out.
      return comments
        .sort((left, right) => right.created_at - left.created_at)
        .slice(0, 500);
    }
    return [];
  };

  const result = await fetchProjectsWorkItems(
    [{ repositories: [{ repoAddress: REPO_ADDRESS }] }],
    fetchEvents,
  );

  assert.equal(result.issues.items.length, 1);
  assert.deepEqual(
    result.issues.items[0].issue.assignees,
    [ASSIGNEE],
    "assignee evicted from the comment window must be restored by the dedicated assignment query",
  );
});

test("a failed assignment query surfaces as a failed section instead of silent loss", async () => {
  const issue = makeIssue();
  const fetchEvents = async (filter) => {
    if (filter.kinds?.includes(1621)) return [issue];
    if (filter["#t"]) throw new Error("relay hiccup");
    return [];
  };

  const result = await fetchProjectsWorkItems(
    [{ repositories: [{ repoAddress: REPO_ADDRESS }] }],
    fetchEvents,
  );

  assert.ok(result.issues.failedSections.includes("assignments"));
});
