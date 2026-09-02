import assert from "node:assert/strict";
import test from "node:test";

import { ProjectCanvasBrokerError } from "./projectCanvasBroker.ts";
import { createProjectCanvasRpcSession } from "./projectCanvasRpc.ts";
import {
  PROJECT_CANVAS_COMMAND_RATE_LIMIT,
  PROJECT_CANVAS_MAX_CONCURRENT_SUBSCRIPTIONS,
  PROJECT_CANVAS_OPEN_RATE_LIMIT,
} from "./projectCanvasProtocol.ts";

const LOAD_ID = "0123456789abcdef0123456789abcdef";
const NONCE = "fedcba9876543210fedcba9876543210";

function message(fields) {
  return { loadId: LOAD_ID, nonce: NONCE, protocolVersion: 1, ...fields };
}

function fakeBroker(overrides = {}) {
  const pushers = new Map();
  return {
    pushers,
    command: async () => {},
    open: async () => {},
    query: async () => ({ data: [], status: "ready" }),
    subscribe: (name, _params, _capabilities, onUpdate) => {
      pushers.set(name, onUpdate);
      onUpdate({ data: [], status: "ready" });
      return () => {
        pushers.delete(name);
      };
    },
    ...overrides,
  };
}

function createSession(broker, overrides = {}) {
  const sent = [];
  const settled = [];
  const session = createProjectCanvasRpcSession({
    broker,
    capabilities: ["project.tasks.read"],
    loadId: LOAD_ID,
    nonce: NONCE,
    now: () => 0,
    onCommandSettled: (name, error) => settled.push({ error, name }),
    post: (payload) => sent.push(payload),
    ...overrides,
  });
  return { sent, session, settled };
}

async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

test("queries answer with the session envelope and broker errors map to rpc errors", async () => {
  const broker = fakeBroker({
    query: async (name) => {
      if (name === "bad") {
        throw new ProjectCanvasBrokerError("forbidden", "No capability.");
      }
      return { data: [1], status: "ready" };
    },
  });
  const { sent, session } = createSession(broker);
  session.handle(
    message({ query: { name: "ok" }, queryId: "q-1", type: "canvas.query" }),
  );
  session.handle(
    message({ query: { name: "bad" }, queryId: "q-2", type: "canvas.query" }),
  );
  await flush();
  assert.deepEqual(sent, [
    {
      loadId: LOAD_ID,
      nonce: NONCE,
      protocolVersion: 1,
      queryId: "q-1",
      result: { data: [1], status: "ready" },
      type: "host.queryResult",
    },
    {
      error: { code: "forbidden", message: "No capability." },
      loadId: LOAD_ID,
      nonce: NONCE,
      protocolVersion: 1,
      queryId: "q-2",
      type: "host.queryResult",
    },
  ]);
});

test("a missing broker answers every request with unavailable", async () => {
  const { sent, session } = createSession(null);
  session.handle(
    message({ query: { name: "x" }, queryId: "q-1", type: "canvas.query" }),
  );
  session.handle(
    message({
      query: { name: "x" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  session.handle(
    message({
      command: { name: "tasks.setStatus" },
      commandId: "c-1",
      type: "canvas.command",
    }),
  );
  session.handle(message({ openId: "o-1", target: {}, type: "canvas.open" }));
  await flush();
  assert.deepEqual(
    sent.map((payload) => [payload.type, payload.error?.code]),
    [
      ["host.queryResult", "unavailable"],
      ["host.subscriptionEnded", "unavailable"],
      ["host.commandResult", "unavailable"],
      ["host.openResult", "unavailable"],
    ],
  );
});

test("the synchronous initial subscription push is delivered", () => {
  const { sent, session } = createSession(fakeBroker());
  session.handle(
    message({
      query: { name: "project.tasks.list" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  assert.deepEqual(sent, [
    {
      loadId: LOAD_ID,
      nonce: NONCE,
      protocolVersion: 1,
      result: { data: [], status: "ready" },
      subscriptionId: "s-1",
      type: "host.subscriptionUpdate",
    },
  ]);
});

test("duplicate ids, the concurrency cap, and broker throws end subscriptions individually", () => {
  const broker = fakeBroker();
  const { sent, session } = createSession(broker);
  const subscribe = (subscriptionId, name = "project.tasks.list") =>
    session.handle(
      message({
        query: { name },
        subscriptionId,
        type: "canvas.subscribe",
      }),
    );

  subscribe("s-1");
  subscribe("s-1");
  assert.equal(sent.at(-1).type, "host.subscriptionEnded");
  assert.equal(sent.at(-1).error.code, "invalid-params");

  for (
    let index = 1;
    index < PROJECT_CANVAS_MAX_CONCURRENT_SUBSCRIPTIONS;
    index += 1
  ) {
    subscribe(`s-extra-${index}`, `query-${index}`);
  }
  subscribe("s-over");
  assert.equal(sent.at(-1).type, "host.subscriptionEnded");
  assert.equal(sent.at(-1).error.code, "rate-limited");
  assert.equal(sent.at(-1).subscriptionId, "s-over");

  const throwing = createSession(
    fakeBroker({
      subscribe: () => {
        throw new ProjectCanvasBrokerError("unsupported", "Not live.");
      },
    }),
  );
  throwing.session.handle(
    message({
      query: { name: "people.search" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  assert.deepEqual(throwing.sent.at(-1).error, {
    code: "unsupported",
    message: "Not live.",
  });
  // The failed id is free again.
  throwing.session.handle(
    message({
      query: { name: "people.search" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  assert.equal(throwing.sent.at(-1).error.code, "unsupported");
});

test("unsubscribe detaches from the broker and stops updates", () => {
  const broker = fakeBroker();
  const { sent, session } = createSession(broker);
  session.handle(
    message({
      query: { name: "project.tasks.list" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  session.handle(
    message({ subscriptionId: "s-1", type: "canvas.unsubscribe" }),
  );
  assert.equal(broker.pushers.has("project.tasks.list"), false);
  assert.equal(sent.length, 1);
});

test("oversized subscription updates end the subscription with too-large", () => {
  const broker = fakeBroker();
  const { sent, session } = createSession(broker);
  session.handle(
    message({
      query: { name: "project.tasks.list" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  broker.pushers.get("project.tasks.list")?.({
    data: ["x".repeat(70 * 1_024)],
    status: "ready",
  });
  assert.equal(sent.at(-1).type, "host.subscriptionEnded");
  assert.equal(sent.at(-1).error.code, "too-large");
  assert.equal(broker.pushers.has("project.tasks.list"), false);
});

test("commands are rate limited per minute and settle the toast hook", async () => {
  let failNext = false;
  const broker = fakeBroker({
    command: async () => {
      if (failNext) {
        throw new ProjectCanvasBrokerError("not-found", "No such task.");
      }
    },
  });
  const { sent, session, settled } = createSession(broker);
  session.handle(
    message({
      command: { name: "tasks.setStatus" },
      commandId: "c-ok",
      type: "canvas.command",
    }),
  );
  await flush();
  assert.deepEqual(sent.at(-1), {
    commandId: "c-ok",
    loadId: LOAD_ID,
    nonce: NONCE,
    ok: true,
    protocolVersion: 1,
    type: "host.commandResult",
  });

  failNext = true;
  session.handle(
    message({
      command: { name: "tasks.assign" },
      commandId: "c-bad",
      type: "canvas.command",
    }),
  );
  await flush();
  assert.equal(sent.at(-1).error.code, "not-found");
  assert.deepEqual(settled, [
    { error: null, name: "tasks.setStatus" },
    { error: "No such task.", name: "tasks.assign" },
  ]);

  failNext = false;
  for (let index = 2; index < PROJECT_CANVAS_COMMAND_RATE_LIMIT; index += 1) {
    session.handle(
      message({
        command: { name: "tasks.setStatus" },
        commandId: `c-${index}`,
        type: "canvas.command",
      }),
    );
  }
  session.handle(
    message({
      command: { name: "tasks.setStatus" },
      commandId: "c-over",
      type: "canvas.command",
    }),
  );
  await flush();
  const over = sent.find((payload) => payload.commandId === "c-over");
  assert.equal(over.error.code, "rate-limited");
  // Rate-limited commands never reach the settle hook.
  assert.equal(settled.length, PROJECT_CANVAS_COMMAND_RATE_LIMIT);
});

test("opens are rate limited on their own window", async () => {
  const { sent, session } = createSession(fakeBroker());
  for (let index = 0; index < PROJECT_CANVAS_OPEN_RATE_LIMIT; index += 1) {
    session.handle(
      message({
        openId: `o-${index}`,
        target: { id: "channel-1", type: "channel" },
        type: "canvas.open",
      }),
    );
  }
  session.handle(
    message({
      openId: "o-over",
      target: { id: "channel-1", type: "channel" },
      type: "canvas.open",
    }),
  );
  await flush();
  assert.equal(
    sent.filter((payload) => payload.ok === true).length,
    PROJECT_CANVAS_OPEN_RATE_LIMIT,
  );
  const over = sent.find((payload) => payload.openId === "o-over");
  assert.equal(over.error.code, "rate-limited");
});

test("dispose unsubscribes everything and drops late completions", async () => {
  let resolveQuery;
  const broker = fakeBroker({
    query: () =>
      new Promise((resolve) => {
        resolveQuery = resolve;
      }),
  });
  const { sent, session } = createSession(broker);
  session.handle(
    message({
      query: { name: "project.tasks.list" },
      subscriptionId: "s-1",
      type: "canvas.subscribe",
    }),
  );
  session.handle(
    message({ query: { name: "slow" }, queryId: "q-1", type: "canvas.query" }),
  );
  session.dispose();
  assert.equal(broker.pushers.has("project.tasks.list"), false);
  resolveQuery({ data: [], status: "ready" });
  await flush();
  assert.equal(
    sent.some((payload) => payload.type === "host.queryResult"),
    false,
  );
  // Post-dispose messages are ignored entirely.
  session.handle(
    message({ query: { name: "x" }, queryId: "q-2", type: "canvas.query" }),
  );
  await flush();
  assert.equal(sent.length, 1);
});
