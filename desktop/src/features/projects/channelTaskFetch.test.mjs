import assert from "node:assert/strict";
import test from "node:test";

import { fetchChannelTasks, fetchProjectTask } from "./channelTaskFetch.ts";

const OWNER = "a".repeat(64);
const PERSON = "b".repeat(64);
const OTHER = "c".repeat(64);
const ADDRESS = `30617:${OWNER}:repo`;
const CHANNEL = "the-channel";
let nextId = 0;
function event(kind, tags = [], overrides = {}) {
  nextId += 1;
  return {
    id: nextId.toString(16).padStart(64, "0"),
    kind,
    pubkey: OWNER,
    created_at: 100,
    content: "Task",
    tags,
    ...overrides,
  };
}
function issue(overrides = {}) {
  return event(
    1621,
    [
      ["a", ADDRESS],
      ["h", CHANNEL],
      ["subject", "A task"],
    ],
    overrides,
  );
}
function fetcher(events, observed = []) {
  return async (filter) => {
    observed.push(filter);
    return events
      .filter(
        (item) =>
          filter.kinds.includes(item.kind) &&
          (!filter.ids || filter.ids.includes(item.id)) &&
          (filter.until === undefined || item.created_at <= filter.until) &&
          (!filter["#e"] ||
            item.tags.some(
              (tag) => tag[0] === "e" && filter["#e"].includes(tag[1]),
            )),
      )
      .sort((a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id))
      .slice(0, Math.min(filter.limit, 1000));
  };
}

test("selected tasks load by exact id beyond recent history and reduce their state", async () => {
  const root = issue({ created_at: 1 });
  const recent = Array.from({ length: 250 }, () => issue());
  const status = event(1632, [["e", root.id]], { created_at: 110 });
  const assignment = event(1, [
    ["e", root.id],
    ["t", "assignment"],
    ["p", PERSON],
  ]);
  const observed = [];
  const result = await fetchProjectTask(
    ADDRESS,
    root.id.toUpperCase(),
    undefined,
    fetcher([...recent, root, status, assignment], observed),
  );
  assert.equal(result.length, 1);
  assert.equal(result[0].id, root.id);
  assert.equal(result[0].status, "Closed");
  assert.deepEqual(result[0].assignees, [PERSON]);
  assert.deepEqual(observed[0].ids, [root.id]);
});

test("selected tasks reject missing roots, wrong repositories, and incomplete state", async () => {
  const root = issue();
  await assert.rejects(
    fetchProjectTask(ADDRESS, "invalid", undefined, fetcher([])),
    /Invalid task/,
  );
  await assert.rejects(
    fetchProjectTask(ADDRESS, root.id, undefined, fetcher([])),
    /unavailable/,
  );
  await assert.rejects(
    fetchProjectTask("another-repo", root.id, undefined, fetcher([root])),
    /unavailable/,
  );
  await assert.rejects(
    fetchProjectTask(ADDRESS, root.id, undefined, async (filter) => {
      if (filter.kinds.includes(1)) throw new Error("offline");
      return fetcher([root])(filter);
    }),
    /current status and assignments/,
  );
});

test("a task result cap is explicit even when the root scan finishes", async () => {
  const roots = Array.from({ length: 501 }, (_, index) =>
    issue({ created_at: index }),
  );
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher(roots),
  );
  assert.equal(result.issues.length, 500);
  assert.equal(result.rootsComplete, false);
});

test("loads only this channel's known repositories and reduces trusted state", async () => {
  const root = issue();
  const otherChannel = issue({
    tags: [
      ["a", ADDRESS],
      ["h", "elsewhere"],
    ],
  });
  const missingOrigin = issue({ tags: [["a", ADDRESS]] });
  const wrongRepo = issue({
    tags: [
      ["a", `30617:${OTHER}:hidden`],
      ["h", CHANNEL],
    ],
  });
  const assignment = event(1, [
    ["e", root.id],
    ["t", "assignment"],
    ["p", PERSON],
  ]);
  const forgedStatus = event(1632, [["e", root.id]], {
    pubkey: OTHER,
    created_at: 120,
  });
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher([
      root,
      otherChannel,
      missingOrigin,
      wrongRepo,
      assignment,
      forgedStatus,
    ]),
  );
  assert.deepEqual(
    result.issues.map((item) => item.id),
    [root.id],
  );
  assert.deepEqual(result.issues[0].assignees, [PERSON]);
  assert.equal(result.issues[0].status, "Backlog");
  assert.ok(
    result.rootsComplete &&
      result.assignmentsComplete &&
      result.statusesComplete,
  );
});

test("pages past unrelated recent roots without post-filtering before LIMIT", async () => {
  const root = issue({ created_at: 1 });
  const newer = Array.from({ length: 510 }, (_, index) =>
    issue({
      created_at: index + 10,
      tags: [
        ["a", ADDRESS],
        ["h", "elsewhere"],
      ],
    }),
  );
  const observed = [];
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher([...newer, root], observed),
  );
  assert.equal(result.issues[0]?.id, root.id);
  assert.equal(result.rootsComplete, true);
  const rootQueries = observed.filter((filter) => filter.kinds.includes(1621));
  assert.ok(rootQueries.length > 1);
  for (const filter of rootQueries) {
    assert.equal(filter["#a"], undefined);
    assert.equal(filter["#h"], undefined);
  }
});

test("loads old assignments past ordinary comments and applies unassignment", async () => {
  const root = issue({ created_at: 1 });
  const assign = event(
    1,
    [
      ["e", root.id],
      ["t", "assignment"],
      ["p", PERSON],
    ],
    { created_at: 2 },
  );
  const unassign = event(
    1,
    [
      ["e", root.id],
      ["t", "unassignment"],
      ["p", PERSON],
    ],
    { created_at: 3 },
  );
  const comments = Array.from({ length: 510 }, (_, index) =>
    event(1, [["e", root.id]], { created_at: index + 10 }),
  );
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher([root, assign, unassign, ...comments]),
  );
  assert.equal(result.assignmentsComplete, true);
  assert.deepEqual(result.issues[0].assignees, []);
});

test("root p tags are routing, not assignment; trusted close events are respected", async () => {
  const root = issue({
    tags: [
      ["a", ADDRESS],
      ["h", CHANNEL],
      ["p", PERSON],
    ],
  });
  const status = event(1632, [["e", root.id]], { created_at: 120 });
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher([root, status]),
  );
  assert.deepEqual(result.issues[0].assignees, []);
  assert.equal(result.issues[0].status, "Closed");
});

test("a failed root request rejects instead of returning an empty success", async () => {
  await assert.rejects(
    fetchChannelTasks(CHANNEL, [ADDRESS], undefined, async () => {
      throw new Error("offline");
    }),
    /offline/,
  );
});

test("failed assignment and status reads remain visibly incomplete", async () => {
  const root = issue();
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    async (filter) => {
      if (filter.kinds.includes(1621)) return [root];
      throw new Error("state unavailable");
    },
  );
  assert.equal(result.issues.length, 1);
  assert.equal(result.rootsComplete, true);
  assert.equal(result.assignmentsComplete, false);
  assert.equal(result.statusesComplete, false);
});

test("dense seconds are marked incomplete rather than skipping unreachable roots", async () => {
  const roots = Array.from({ length: 1010 }, () =>
    issue({
      tags: [
        ["a", ADDRESS],
        ["h", "elsewhere"],
      ],
    }),
  );
  const observed = [];
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    fetcher(roots, observed),
  );
  assert.deepEqual(result.issues, []);
  assert.equal(result.rootsComplete, false);
  assert.ok(observed.length <= 4);
});

test("large histories stop at the page budget and retain an incomplete result", async () => {
  let calls = 0;
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    async () => {
      calls += 1;
      return Array.from({ length: 500 }, (_, index) =>
        issue({
          created_at: 100000 - calls * 500 - index,
          tags: [
            ["a", ADDRESS],
            ["h", "elsewhere"],
          ],
        }),
      );
    },
  );
  assert.equal(result.rootsComplete, false);
  assert.equal(calls, 10);
});

test("cancellation after a response prevents additional reads and reduction", async () => {
  const controller = new AbortController();
  let calls = 0;
  await assert.rejects(
    fetchChannelTasks(CHANNEL, [ADDRESS], controller.signal, async () => {
      calls += 1;
      controller.abort();
      return [issue()];
    }),
    { name: "AbortError" },
  );
  assert.equal(calls, 1);
});

test("an empty repository scope makes no relay requests", async () => {
  const result = await fetchChannelTasks(CHANNEL, [], undefined, async () => {
    assert.fail("unexpected relay query");
  });
  assert.deepEqual(result.issues, []);
  assert.equal(result.rootsComplete, true);
});

test("state history shares a total request budget across issue batches", async () => {
  const roots = Array.from({ length: 250 }, (_, index) =>
    issue({ created_at: index + 1 }),
  );
  let calls = 0;
  const result = await fetchChannelTasks(
    CHANNEL,
    [ADDRESS],
    undefined,
    async (filter) => {
      calls += 1;
      if (filter.kinds.includes(1621)) return roots;
      const head = filter.until ?? 100_000;
      return Array.from({ length: 500 }, (_, index) =>
        event(filter.kinds[0], [["e", roots[0].id]], {
          created_at: head - index - 1,
        }),
      );
    },
  );
  assert.equal(calls, 24);
  assert.equal(result.rootsComplete, true);
  assert.equal(result.assignmentsComplete, false);
  assert.equal(result.statusesComplete, false);
});
