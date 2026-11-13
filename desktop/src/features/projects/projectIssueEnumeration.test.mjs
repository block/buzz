import assert from "node:assert/strict";
import test from "node:test";

import { fetchProjectIssues } from "./hooks.ts";
import { projectTaskActivity } from "./ui/projectRightPanelContext.ts";

const OWNER = "a".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:ralph-queue`;
const PROJECT = { repoAddress: REPO_ADDRESS };

function issueRoot(index) {
  return {
    id: `${index}`.padStart(64, "0"),
    kind: 1621,
    pubkey: OWNER,
    // Newest-first ordering is what makes a bounded window lose the oldest
    // roots, so index 0 has to be the oldest one.
    created_at: 1_000 + index,
    content: `Task ${index}`,
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", `Task ${index}`],
    ],
  };
}

/** Owner-signed kind:1632, so the root reduces to `Closed`. */
function closedStatus(issue) {
  return {
    id: `${issue.created_at}`.padStart(64, "e"),
    kind: 1632,
    pubkey: OWNER,
    created_at: issue.created_at + 1,
    content: "",
    tags: [
      ["e", issue.id, "", "root"],
      ["a", REPO_ADDRESS],
    ],
  };
}

/**
 * Stands in for the relay: newest-first (`created_at DESC, id ASC`), honouring
 * `limit`, `until`, and `since` the way `crates/buzz-db/src/event.rs` does.
 */
function makeRelay(events) {
  return async function fetchEvents(filter) {
    const kinds = new Set(filter.kinds);
    return events
      .filter(
        (event) =>
          kinds.has(event.kind) &&
          (filter.until === undefined || event.created_at <= filter.until) &&
          (filter.since === undefined || event.created_at >= filter.since),
      )
      .sort((a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id))
      .slice(0, filter.limit);
  };
}

test("project issue loader returns every root past the 200-event window", async () => {
  const roots = Array.from({ length: 201 }, (_, index) => issueRoot(index));
  const relay = makeRelay([...roots, closedStatus(roots[0])]);

  const issues = await fetchProjectIssues(PROJECT, relay);

  assert.deepEqual(projectTaskActivity(issues), {
    total: 201,
    active: 200,
    completed: 1,
  });
});

test("a newer active root does not evict the oldest completed one", async () => {
  const roots = Array.from({ length: 201 }, (_, index) => issueRoot(index));
  const relay = makeRelay([...roots, issueRoot(201), closedStatus(roots[0])]);

  const issues = await fetchProjectIssues(PROJECT, relay);

  assert.deepEqual(projectTaskActivity(issues), {
    total: 202,
    active: 201,
    completed: 1,
  });
});

test("issue roots deeper than one page are drained by the cursor", async () => {
  const roots = Array.from({ length: 501 }, (_, index) => issueRoot(index));
  const relay = makeRelay([...roots, closedStatus(roots[0])]);
  let issuePages = 0;
  const counted = async (filter) => {
    if (filter.kinds.includes(1621)) issuePages += 1;
    return relay(filter);
  };

  const issues = await fetchProjectIssues(PROJECT, counted);

  assert.ok(issuePages > 1, `expected more than one page, got ${issuePages}`);
  assert.deepEqual(projectTaskActivity(issues), {
    total: 501,
    active: 500,
    completed: 1,
  });
});
