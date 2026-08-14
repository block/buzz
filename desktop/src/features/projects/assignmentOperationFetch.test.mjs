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
function makeAssignment(id, createdAt, issueId = ISSUE_ID) {
  return {
    id,
    kind: 1,
    pubkey: REPO_OWNER,
    created_at: createdAt,
    content: "Assigned",
    tags: [
      ["e", issueId, "", "root"],
      ["a", REPO_ADDRESS],
      ["p", ASSIGNEE],
      ["t", "assignment"],
    ],
  };
}

function makeComment(id, createdAt, issueId = ISSUE_ID) {
  return {
    id,
    kind: 1,
    pubkey: REPO_OWNER,
    created_at: createdAt,
    content: `Comment ${id.slice(0, 4)}`,
    tags: [
      ["e", issueId, "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

function eventId(index) {
  return index.toString(16).padStart(64, "0");
}

/**
 * Relay model matching production semantics (`filter_to_query_params` +
 * `filter_fully_pushable` in `crates/buzz-relay/src/handlers/req.rs`):
 *
 * 1. SQL applies kinds / `#e` / `since` / `until` (inclusive), orders
 *    `(created_at DESC, id ASC)`, and cuts to `LIMIT` — clamped to 1,000.
 * 2. `#t` / `#a` are applied in Rust AFTER the SQL LIMIT.
 *
 * Step 2 is the trap the round-3 review caught: a filter that carries `#t`
 * gets the newest N kind-1 candidates first, then loses tag mismatches, so a
 * short page does NOT mean exhaustion. Any regression back to `#t`-reliant
 * fetching fails these tests.
 */
function makeRelayModel(events) {
  const calls = [];
  const fetchEvents = async (filter) => {
    calls.push(filter);
    const limit = Math.min(filter.limit ?? 1_000, 1_000);
    // SQL phase: pushed constraints only.
    const candidates = events
      .filter((event) => {
        if (filter.kinds && !filter.kinds.includes(event.kind)) return false;
        if (
          filter["#e"] &&
          !event.tags.some(
            (tag) => tag[0] === "e" && filter["#e"].includes(tag[1]),
          )
        ) {
          return false;
        }
        if (filter.until !== undefined && event.created_at > filter.until) {
          return false;
        }
        if (filter.since !== undefined && event.created_at < filter.since) {
          return false;
        }
        return true;
      })
      .sort((left, right) =>
        right.created_at !== left.created_at
          ? right.created_at - left.created_at
          : left.id < right.id
            ? -1
            : 1,
      )
      .slice(0, limit);
    // Rust post-filter phase: tag constraints applied AFTER the LIMIT.
    return candidates.filter((event) => {
      if (
        filter["#t"] &&
        !event.tags.some(
          (tag) => tag[0] === "t" && filter["#t"].includes(tag[1]),
        )
      ) {
        return false;
      }
      if (
        filter["#a"] &&
        !event.tags.some(
          (tag) => tag[0] === "a" && filter["#a"].includes(tag[1]),
        )
      ) {
        return false;
      }
      return true;
    });
  };
  return { calls, fetchEvents };
}

// ── fetchAssignmentOperationEvents vs. real relay query semantics ────────────

test("finds an assignment buried behind 600 newer comments on the real relay model", async () => {
  // The round-3 adversarial case: one old assignment, then 600 newer comments
  // on the same issue. A `#t`-carrying filter sees the newest 500 candidates
  // post-filtered to zero and falsely declares exhaustion (0/1). The
  // `#e`-keyed walk must return 1/1.
  const assignment = makeAssignment(eventId(9_999), 200);
  const comments = Array.from({ length: 600 }, (_, index) =>
    makeComment(eventId(index + 1), 1_000 + index),
  );
  const { calls, fetchEvents } = makeRelayModel([assignment, ...comments]);

  const events = await fetchAssignmentOperationEvents([ISSUE_ID], fetchEvents);

  assert.deepEqual(
    events.map((event) => event.id),
    [eventId(9_999)],
    "the buried assignment must survive the bounded page",
  );
  assert.ok(calls.length >= 2, "must page past the first bounded window");
  for (const filter of calls) {
    assert.equal(
      filter["#t"],
      undefined,
      "the filter must carry only SQL-pushed constraints — `#t` is post-filtered after LIMIT",
    );
    assert.equal(filter["#a"], undefined, "`#a` is post-filtered after LIMIT");
  }
});

test("paginates to exhaustion across several full pages", async () => {
  // 1,203 operations spread across distinct seconds — three pages at the
  // 500-event window.
  const operations = Array.from({ length: 1_203 }, (_, index) =>
    makeAssignment(eventId(index + 1), 1_000_000 + index),
  );
  const { calls, fetchEvents } = makeRelayModel(operations);

  const events = await fetchAssignmentOperationEvents([ISSUE_ID], fetchEvents);

  assert.equal(events.length, 1_203, "every operation must be loaded");
  assert.ok(calls.length >= 3, "must page past the 500-event window");
});

test("escapes a second denser than one page by widening to the relay clamp", async () => {
  // 700 externally signed operations sharing one created_at second: an
  // inclusive `until` cursor alone can never advance past the first 500.
  // The loop must widen to the relay's 1,000-row clamp and load all 700.
  const operations = Array.from({ length: 700 }, (_, index) =>
    makeAssignment(eventId(index + 1), 1_000),
  );
  const { fetchEvents } = makeRelayModel(operations);

  const events = await fetchAssignmentOperationEvents([ISSUE_ID], fetchEvents);

  assert.equal(events.length, 700, "same-second density must not drop events");
});

test("reports a second denser than the relay clamp instead of silently dropping", async () => {
  // 1,100 events in one second exceeds the relay's 1,000-row page clamp —
  // unreachable through NIP-01 filters. That must surface as an error (the
  // caller shows a failed assignments section), never as silent loss.
  const operations = Array.from({ length: 1_100 }, (_, index) =>
    makeAssignment(eventId(index + 1), 1_000),
  );
  const { fetchEvents } = makeRelayModel(operations);

  await assert.rejects(
    fetchAssignmentOperationEvents([ISSUE_ID], fetchEvents),
    /full relay page/,
  );
});

test("skips the relay for zero issues", async () => {
  const events = await fetchAssignmentOperationEvents([], async () => {
    throw new Error("must not query the relay");
  });
  assert.deepEqual(events, []);
});

test("chunks large issue sets and dedupes operations across chunks", async () => {
  // 150 issues → two `#e` chunks at the 100-id chunk size.
  const issueIds = Array.from({ length: 150 }, (_, index) =>
    eventId(5_000 + index),
  );
  const operations = issueIds.map((issueId, index) =>
    makeAssignment(eventId(index + 1), 2_000 + index, issueId),
  );
  const { calls, fetchEvents } = makeRelayModel(operations);

  const events = await fetchAssignmentOperationEvents(issueIds, fetchEvents);

  assert.equal(events.length, 150);
  assert.equal(calls.length, 2, "150 issues must fan out as two #e chunks");
  assert.ok(calls.every((filter) => filter["#e"].length <= 100));
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
// newer unrelated comments. The dedicated issue-keyed exhaustive query must
// restore it — against the real relay's query semantics.

test("an assignment older than 600 newer comments still reduces to an assignee", async () => {
  const issue = makeIssue();
  const assignment = makeAssignment(eventId(9_999), 200);
  const comments = Array.from({ length: 600 }, (_, index) =>
    makeComment(eventId(index + 1), 1_000 + index),
  );
  const { fetchEvents } = makeRelayModel([issue, assignment, ...comments]);

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
    if (filter["#e"]) throw new Error("relay hiccup");
    return [];
  };

  const result = await fetchProjectsWorkItems(
    [{ repositories: [{ repoAddress: REPO_ADDRESS }] }],
    fetchEvents,
  );

  assert.ok(result.issues.failedSections.includes("assignments"));
});
