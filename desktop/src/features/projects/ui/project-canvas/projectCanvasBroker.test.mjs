import assert from "node:assert/strict";
import test from "node:test";

import {
  PROJECT_CANVAS_DM_MESSAGE_MAX_LENGTH,
  PROJECT_CANVAS_LOOKUP_PUBKEY_LIMIT,
  ProjectCanvasBroker,
  ProjectCanvasBrokerError,
} from "./projectCanvasBroker.ts";

const ALL_CAPABILITIES = [
  "project.metadata.read",
  "project.channels.read",
  "project.reviews.read",
  "project.tasks.read",
  "project.people.read",
  "project.tasks.write",
  "app.open",
  "app.dm.send",
];

function channel(id, name, relationship = "home") {
  return {
    description: "",
    id,
    lastMessageAt: null,
    memberCount: 1,
    name,
    people: [],
    relationship,
    topic: null,
  };
}

function task(id, overrides = {}) {
  return {
    assignees: [],
    category: "Bug",
    commentCount: 0,
    displayId: `#${id.slice(0, 4)}`,
    id,
    status: "Triage",
    title: `Task ${id.slice(0, 4)}`,
    updatedAt: 1,
    ...overrides,
  };
}

function review(id, overrides = {}) {
  return {
    author: "a".repeat(64),
    branch: null,
    displayId: id.slice(0, 8),
    id,
    status: "Open",
    title: `Review ${id.slice(0, 4)}`,
    updatedAt: 1,
    ...overrides,
  };
}

function createBroker(overrides = {}) {
  const calls = {
    commands: [],
    directMessages: [],
    lookups: [],
    opens: [],
    searches: [],
  };
  const broker = new ProjectCanvasBroker({
    lookupPeople: async (pubkeys) => {
      calls.lookups.push(pubkeys);
      return pubkeys.map((pubkey) => ({
        avatarDataUrl: null,
        displayName: `Person ${pubkey.slice(0, 4)}`,
        isAgent: false,
        pubkey,
      }));
    },
    openTarget: async (target) => {
      calls.opens.push(target);
    },
    runTaskCommand: async (command) => {
      calls.commands.push(command);
    },
    searchPeople: async (query, limit) => {
      calls.searches.push({ limit, query });
      return [];
    },
    sendDirectMessage: async (recipient, message) => {
      calls.directMessages.push({ message, recipient });
    },
    ...overrides,
  });
  return { broker, calls };
}

const TASK_ID = "b".repeat(64);
const REVIEW_ID = "c".repeat(64);

function readySources(overrides = {}) {
  return {
    channels: {
      data: [
        channel("channel-1", "general"),
        channel("channel-2", "docs", "related"),
      ],
      status: "ready",
    },
    project: { data: { name: "proj" }, status: "ready" },
    reviews: {
      data: [review(REVIEW_ID), review("d".repeat(64), { status: "Merged" })],
      status: "ready",
    },
    tasks: {
      data: [
        task(TASK_ID),
        task("e".repeat(64), { assignees: ["f".repeat(64)], status: "Done" }),
      ],
      status: "ready",
    },
    ...overrides,
  };
}

async function rejectionCode(promise) {
  try {
    await promise;
    return null;
  } catch (error) {
    assert.ok(error instanceof ProjectCanvasBrokerError);
    return error.code;
  }
}

test("queries require their mapped capability and reject unknown names", async () => {
  const { broker } = createBroker();
  broker.setSources(readySources());
  assert.equal(
    await rejectionCode(broker.query("project.tasks.list", {}, [])),
    "forbidden",
  );
  assert.equal(
    await rejectionCode(broker.query("nope", {}, ALL_CAPABILITIES)),
    "unsupported",
  );
  const result = await broker.query("project.tasks.list", {}, ALL_CAPABILITIES);
  assert.equal(result.status, "ready");
  assert.equal(result.data.length, 2);
});

test("list queries filter and bound rows without widening scope", async () => {
  const { broker } = createBroker();
  broker.setSources(readySources());

  const home = await broker.query(
    "project.channels.list",
    { relationship: "home" },
    ALL_CAPABILITIES,
  );
  assert.deepEqual(
    home.data.map((row) => row.id),
    ["channel-1"],
  );

  const merged = await broker.query(
    "project.reviews.list",
    { status: "Merged" },
    ALL_CAPABILITIES,
  );
  assert.deepEqual(
    merged.data.map((row) => row.status),
    ["Merged"],
  );

  const assigned = await broker.query(
    "project.tasks.list",
    { assignee: "F".repeat(64) },
    ALL_CAPABILITIES,
  );
  assert.equal(assigned.data.length, 1);
  assert.equal(assigned.data[0].status, "Done");

  const limited = await broker.query(
    "project.tasks.list",
    { limit: 1 },
    ALL_CAPABILITIES,
  );
  assert.equal(limited.data.length, 1);

  assert.equal(
    await rejectionCode(
      broker.query("project.tasks.list", { limit: 51 }, ALL_CAPABILITIES),
    ),
    "invalid-params",
  );
  assert.equal(
    await rejectionCode(
      broker.query("project.tasks.list", { repo: "other" }, ALL_CAPABILITIES),
    ),
    "invalid-params",
  );
});

test("people lookups dedupe, lowercase, and bound their pubkey batch", async () => {
  const { broker, calls } = createBroker();
  const result = await broker.query(
    "people.lookup",
    { pubkeys: ["A".repeat(64), "a".repeat(64)] },
    ALL_CAPABILITIES,
  );
  assert.equal(result.status, "ready");
  assert.deepEqual(calls.lookups, [["a".repeat(64)]]);

  assert.equal(
    await rejectionCode(
      broker.query(
        "people.lookup",
        {
          pubkeys: Array.from(
            { length: PROJECT_CANVAS_LOOKUP_PUBKEY_LIMIT + 1 },
            (_, index) => index.toString(16).padStart(64, "0"),
          ),
        },
        ALL_CAPABILITIES,
      ),
    ),
    "invalid-params",
  );
});

test("people search bounds the query text", async () => {
  const { broker, calls } = createBroker();
  await broker.query("people.search", { query: "rev" }, ALL_CAPABILITIES);
  assert.deepEqual(calls.searches, [{ limit: 8, query: "rev" }]);
  assert.equal(
    await rejectionCode(
      broker.query(
        "people.search",
        { query: "x".repeat(65) },
        ALL_CAPABILITIES,
      ),
    ),
    "invalid-params",
  );
});

test("subscriptions push immediately, dedupe unchanged results, and stop cleanly", () => {
  const { broker } = createBroker();
  broker.setSources(readySources());
  const updates = [];
  const unsubscribe = broker.subscribe(
    "project.tasks.list",
    {},
    ALL_CAPABILITIES,
    (result) => updates.push(result),
  );
  assert.equal(updates.length, 1);
  assert.equal(updates[0].status, "ready");

  // Same content — no push.
  broker.setSources(readySources());
  assert.equal(updates.length, 1);

  broker.setSources(
    readySources({
      tasks: { data: [task(TASK_ID, { status: "Done" })], status: "ready" },
    }),
  );
  assert.equal(updates.length, 2);
  assert.equal(updates[1].data[0].status, "Done");

  unsubscribe();
  broker.setSources(readySources());
  assert.equal(updates.length, 2);

  assert.throws(
    () =>
      broker.subscribe(
        "people.search",
        { query: "x" },
        ALL_CAPABILITIES,
        () => {},
      ),
    (error) =>
      error instanceof ProjectCanvasBrokerError && error.code === "unsupported",
  );
});

test("commands require the write capability and resolve tasks from host sources", async () => {
  const { broker, calls } = createBroker();
  broker.setSources(readySources());

  assert.equal(
    await rejectionCode(
      broker.command("tasks.setStatus", { id: TASK_ID, status: "done" }, [
        "project.tasks.read",
      ]),
    ),
    "forbidden",
  );
  assert.equal(
    await rejectionCode(
      broker.command("tasks.explode", { id: TASK_ID }, ALL_CAPABILITIES),
    ),
    "unsupported",
  );
  assert.equal(
    await rejectionCode(
      broker.command(
        "tasks.setStatus",
        { id: "9".repeat(64), status: "done" },
        ALL_CAPABILITIES,
      ),
    ),
    "not-found",
  );

  await broker.command(
    "tasks.setStatus",
    { id: TASK_ID, status: "done" },
    ALL_CAPABILITIES,
  );
  await broker.command(
    "tasks.assign",
    { assignee: "F".repeat(64), id: TASK_ID },
    ALL_CAPABILITIES,
  );
  assert.equal(calls.commands.length, 2);
  assert.equal(calls.commands[0].name, "tasks.setStatus");
  assert.equal(calls.commands[0].status, "done");
  assert.equal(calls.commands[0].task.id, TASK_ID);
  assert.equal(calls.commands[1].assignee, "f".repeat(64));
});

test("dm.send requires its capability, bounds the message, and normalizes the recipient", async () => {
  const { broker, calls } = createBroker();
  broker.setSources(readySources());

  assert.equal(
    await rejectionCode(
      broker.command(
        "dm.send",
        { message: "hello", pubkey: "a".repeat(64) },
        ALL_CAPABILITIES.filter((capability) => capability !== "app.dm.send"),
      ),
    ),
    "forbidden",
  );
  assert.equal(
    await rejectionCode(
      broker.command("dm.send", { message: "hello" }, ALL_CAPABILITIES),
    ),
    "invalid-params",
  );
  assert.equal(
    await rejectionCode(
      broker.command(
        "dm.send",
        { message: "   ", pubkey: "a".repeat(64) },
        ALL_CAPABILITIES,
      ),
    ),
    "invalid-params",
  );
  assert.equal(
    await rejectionCode(
      broker.command(
        "dm.send",
        {
          message: "x".repeat(PROJECT_CANVAS_DM_MESSAGE_MAX_LENGTH + 1),
          pubkey: "a".repeat(64),
        },
        ALL_CAPABILITIES,
      ),
    ),
    "invalid-params",
  );
  assert.equal(
    await rejectionCode(
      broker.command(
        "dm.send",
        { extra: true, message: "hello", pubkey: "a".repeat(64) },
        ALL_CAPABILITIES,
      ),
    ),
    "invalid-params",
  );
  assert.equal(calls.directMessages.length, 0);

  await broker.command(
    "dm.send",
    { message: "  Checking in  ", pubkey: "A".repeat(64) },
    ALL_CAPABILITIES,
  );
  assert.deepEqual(calls.directMessages, [
    { message: "Checking in", recipient: "a".repeat(64) },
  ]);
});

test("open targets are validated against host sources before navigation", async () => {
  const { broker, calls } = createBroker();
  broker.setSources(readySources());

  assert.equal(
    await rejectionCode(broker.open({ id: "channel-1", type: "channel" }, [])),
    "forbidden",
  );
  assert.equal(
    await rejectionCode(
      broker.open({ id: "outside-channel", type: "channel" }, ALL_CAPABILITIES),
    ),
    "not-found",
  );
  assert.equal(
    await rejectionCode(
      broker.open({ id: "9".repeat(64), type: "review" }, ALL_CAPABILITIES),
    ),
    "not-found",
  );
  assert.equal(
    await rejectionCode(broker.open({ type: "everything" }, ALL_CAPABILITIES)),
    "invalid-params",
  );

  await broker.open({ id: "channel-1", type: "channel" }, ALL_CAPABILITIES);
  await broker.open({ pubkey: "A".repeat(64), type: "user" }, ALL_CAPABILITIES);
  await broker.open({ id: TASK_ID, type: "task" }, ALL_CAPABILITIES);
  await broker.open(
    { id: REVIEW_ID.toUpperCase(), type: "review" },
    ALL_CAPABILITIES,
  );
  assert.equal(calls.opens.length, 4);
  assert.equal(calls.opens[1].pubkey, "a".repeat(64));
});
