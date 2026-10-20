import assert from "node:assert/strict";
import test from "node:test";

import { fetchProjectTask, normalizeRepoAddress } from "./fetchProjectTask.ts";

const OWNER = "a".repeat(64);
const READER = "b".repeat(64);
const STRANGER = "c".repeat(64);
const ISSUE_ID = "1".repeat(64);
const REPO_ADDRESS = `30617:${OWNER}:relay`;

function issueEvent(overrides = {}) {
  return {
    id: ISSUE_ID,
    kind: 1621,
    pubkey: OWNER,
    created_at: 100,
    content: "Ship the recap",
    tags: [
      ["a", REPO_ADDRESS],
      ["subject", "Ship the recap"],
    ],
    ...overrides,
  };
}

function assignment(id, signer, createdAt, label = "assignment") {
  return {
    id,
    kind: 1,
    pubkey: signer,
    created_at: createdAt,
    content: "",
    tags: [
      ["e", ISSUE_ID, "", "root"],
      ["a", REPO_ADDRESS],
      ["p", READER],
      ["t", label],
    ],
  };
}

function relay({ issues = [issueEvent()], statuses = [], comments = [] }) {
  const filters = [];
  const fetchEvents = async (filter) => {
    filters.push(filter);
    if (filter.kinds.includes(1621)) {
      return issues.filter((event) => filter.ids.includes(event.id));
    }
    if (filter.kinds.includes(1630)) {
      return statuses
        .filter(
          (event) =>
            filter.until === undefined || event.created_at <= filter.until,
        )
        .sort((a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id))
        .slice(0, filter.limit);
    }
    return comments;
  };
  return { fetchEvents, filters };
}

test("reduces the exact task with exhaustive assignment operations", async () => {
  const { fetchEvents, filters } = relay({
    comments: [assignment("2".repeat(64), OWNER, 110)],
  });
  const task = await fetchProjectTask(
    ISSUE_ID.toUpperCase(),
    REPO_ADDRESS,
    fetchEvents,
  );
  assert.equal(task?.id, ISSUE_ID);
  assert.deepEqual(task?.assignees, [READER]);
  assert.equal(task?.status, "Backlog");
  assert.deepEqual(filters[0], { kinds: [1621], ids: [ISSUE_ID], limit: 1 });
  assert.deepEqual(filters[1]["#e"], [ISSUE_ID]);
});

const rejected = [
  ["the relay has no such task", { issues: [] }, REPO_ADDRESS],
  ["the task lives in another repository", {}, `30617:${OWNER}:other`],
  [
    "the repository identifier differs only by case",
    {},
    `30617:${OWNER}:Relay`,
  ],
  ["the link names a malformed coordinate", {}, "30617:not-hex:relay"],
  [
    "a different event kind answers the id",
    { issues: [issueEvent({ kind: 1 })] },
    REPO_ADDRESS,
  ],
];

for (const [name, relayState, address] of rejected) {
  test(`returns null when ${name}`, async () => {
    const { fetchEvents } = relay(relayState);
    assert.equal(await fetchProjectTask(ISSUE_ID, address, fetchEvents), null);
  });
}

test("ignores assignments and statuses from untrusted signers", async () => {
  const { fetchEvents } = relay({
    comments: [assignment("3".repeat(64), STRANGER, 110)],
    statuses: [
      {
        id: "4".repeat(64),
        kind: 1632,
        pubkey: STRANGER,
        created_at: 120,
        content: "",
        tags: [["e", ISSUE_ID]],
      },
    ],
  });
  const task = await fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents);
  assert.deepEqual(task?.assignees, []);
  assert.equal(task?.status, "Backlog");
});

test("applies trusted closing status and later unassignment", async () => {
  const { fetchEvents } = relay({
    comments: [
      assignment("5".repeat(64), OWNER, 110),
      assignment("6".repeat(64), OWNER, 130, "unassignment"),
    ],
    statuses: [
      {
        id: "7".repeat(64),
        kind: 1631,
        pubkey: OWNER,
        created_at: 120,
        content: "",
        tags: [["e", ISSUE_ID]],
      },
    ],
  });
  const task = await fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents);
  assert.deepEqual(task?.assignees, []);
  assert.equal(task?.status, "Done");
});

test("rejects instead of guessing when assignment history fails", async () => {
  const fetchEvents = async (filter) => {
    if (filter.kinds.includes(1621)) return [issueEvent()];
    if (filter.kinds.includes(1630)) return [];
    throw new Error("relay unavailable");
  };
  await assert.rejects(
    fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents),
    /relay unavailable/,
  );
});

test("repository owner hex is case-insensitive, identifier is exact", async () => {
  assert.equal(
    normalizeRepoAddress(`30617:${OWNER.toUpperCase()}:Relay`),
    `30617:${OWNER}:Relay`,
  );
  const { fetchEvents } = relay({});
  const task = await fetchProjectTask(
    ISSUE_ID,
    `30617:${OWNER.toUpperCase()}:relay`,
    fetchEvents,
  );
  assert.equal(task?.id, ISSUE_ID);
});

function status(n, signer, createdAt, kind = 1630) {
  return {
    id: n.toString(16).padStart(64, "0"),
    kind,
    pubkey: signer,
    created_at: createdAt,
    content: "",
    tags: [["e", ISSUE_ID]],
  };
}

test("pages past untrusted status floods to the latest authorized status", async () => {
  const flood = Array.from({ length: 700 }, (_, i) =>
    status(0x1000 + i, STRANGER, 1_000 + i),
  );
  const { fetchEvents, filters } = relay({
    statuses: [status(0x10, OWNER, 500, 1632), ...flood],
  });
  const task = await fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents);
  assert.equal(task?.status, "Closed");
  assert.ok(
    filters.some(
      (filter) => filter.kinds.includes(1630) && filter.until !== undefined,
    ),
  );
});

test("rejects a status history larger than the explicit cap", async () => {
  const flood = Array.from({ length: 2_100 }, (_, i) =>
    status(0x1000 + i, STRANGER, 1_000 + i),
  );
  const { fetchEvents } = relay({ statuses: flood });
  await assert.rejects(
    fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents),
    /too large to verify/,
  );
});

test("rejects a single second denser than a status page", async () => {
  const flood = Array.from({ length: 600 }, (_, i) =>
    status(0x1000 + i, STRANGER, 1_000),
  );
  const { fetchEvents } = relay({ statuses: flood });
  await assert.rejects(
    fetchProjectTask(ISSUE_ID, REPO_ADDRESS, fetchEvents),
    /too large to verify/,
  );
});
