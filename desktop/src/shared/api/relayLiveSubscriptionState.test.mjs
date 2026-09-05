import assert from "node:assert/strict";
import test from "node:test";

let fakeNow = 0;
let nextTimerId = 1;
const pendingTimers = new Map();
const sentFrames = [];

globalThis.window = {
  setTimeout: (fn, ms) => {
    const id = nextTimerId++;
    pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
    return id;
  },
  clearTimeout: (id) => pendingTimers.delete(id),
  __TAURI_INTERNALS__: {
    invoke: async (command, args) => {
      if (command === "plugin:websocket|send") sentFrames.push(args);
    },
  },
};
Date.now = () => fakeNow;

const { RelayClient } = await import("./relayClientSession.ts");
const { receiveLifecycle } = await import("@/features/agents/desktopLifecycle");
const { resetRateLimitGate } = await import("./relayRateLimitGate.ts");

function resetHarness() {
  fakeNow = 0;
  nextTimerId = 1;
  pendingTimers.clear();
  sentFrames.length = 0;
  resetRateLimitGate();
}

function connectedClient() {
  const client = new RelayClient();
  client.wsId = 7;
  return client;
}

function sentProtocolFrames(type) {
  return sentFrames
    .map(({ message }) => JSON.parse(message.data))
    .filter((frame) => frame[0] === type);
}

async function flushUntil(predicate, attempts = 40) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail("condition did not become true before the microtask limit");
}

async function flushMicrotasks(attempts = 10) {
  for (let attempt = 0; attempt < attempts; attempt++) await Promise.resolve();
}

function tickTo(time) {
  fakeNow = time;
  for (;;) {
    const due = [...pendingTimers.entries()].filter(
      ([, timer]) => timer.fireAt <= fakeNow,
    );
    if (!due.length) return;
    for (const [id, timer] of due) {
      if (!pendingTimers.delete(id)) continue;
      timer.fn();
    }
  }
}

function deliver(client, frame) {
  return client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.connectionGeneration,
  );
}

async function openLive(client, options, onEvent = () => {}, onReady) {
  const opened = client.subscribeLive(
    { kinds: [50182, 50180], authors: ["owner"], limit: 0 },
    onEvent,
    onReady,
    5000,
    options,
  );
  await flushUntil(() => sentProtocolFrames("REQ").length > 0);
  const subId = sentProtocolFrames("REQ").at(-1)[1];
  return { opened, subId };
}

test("persistent state reports timeout, late EOSE, then CLOSED through RelayClient", async () => {
  resetHarness();
  const client = connectedClient();
  const states = [];
  const readiness = [];
  const { opened, subId } = await openLive(
    client,
    { closedRecovery: "explicit", onState: (state) => states.push(state) },
    () => {},
    (state) => readiness.push(state),
  );

  tickTo(5000);
  const close = await opened;
  assert.deepEqual(states, ["timeout"]);
  assert.deepEqual(readiness, ["timeout"]);

  await deliver(client, ["EOSE", subId]);
  await deliver(client, ["CLOSED", subId, "restricted: access revoked"]);

  assert.deepEqual(states, ["timeout", "eose", "closed"]);
  assert.deepEqual(readiness, ["timeout", "eose"]);
  assert.equal(client.subscriptions.has(subId), false);
  await close();
});

for (const [label, message] of [
  ["terminal", "restricted: access revoked"],
  ["retryable", "error: storage temporarily unavailable"],
]) {
  test(`explicit recovery retires an EOSE-ready ${label} CLOSED without re-REQ`, async () => {
    resetHarness();
    const client = connectedClient();
    const states = [];
    const { opened, subId } = await openLive(client, {
      closedRecovery: "explicit",
      onState: (state) => states.push(state),
    });
    await deliver(client, ["EOSE", subId]);
    const close = await opened;

    await deliver(client, ["CLOSED", subId, message]);
    tickTo(60_000);
    await Promise.resolve();

    assert.deepEqual(states, ["eose", "closed"]);
    assert.equal(client.subscriptions.has(subId), false);
    assert.equal(sentProtocolFrames("REQ").length, 1);
    await close();
  });
}

test("connection reset reports CLOSED and retires only explicit-recovery subscriptions", async () => {
  resetHarness();
  const client = connectedClient();
  const explicitStates = [];
  const explicit = await openLive(client, {
    closedRecovery: "explicit",
    onState: (state) => explicitStates.push(state),
  });
  await deliver(client, ["EOSE", explicit.subId]);
  const closeExplicit = await explicit.opened;

  const ordinary = await openLive(client);
  await deliver(client, ["EOSE", ordinary.subId]);
  const closeOrdinary = await ordinary.opened;

  client.resetConnection(new Error("fixture connection reset"));

  assert.deepEqual(explicitStates, ["eose", "closed"]);
  assert.equal(client.subscriptions.has(explicit.subId), false);
  assert.equal(
    client.subscriptions.has(ordinary.subId),
    true,
    "default live subscribers retain shared reconnect recovery",
  );

  await closeExplicit();
  await closeOrdinary();
  client.disconnect();
});

test("default live subscribers retain shared retry after retryable CLOSED", async () => {
  resetHarness();
  const client = connectedClient();
  const ordinary = await openLive(client);
  await deliver(client, ["EOSE", ordinary.subId]);
  const close = await ordinary.opened;

  await deliver(client, [
    "CLOSED",
    ordinary.subId,
    "error: storage temporarily unavailable",
  ]);
  assert.equal(client.subscriptions.has(ordinary.subId), true);

  tickTo(1000);
  await flushUntil(() => sentProtocolFrames("REQ").length === 2);
  assert.equal(client.subscriptions.has(ordinary.subId), true);

  await close();
});

test("real explicit CLOSED fences queued lifecycle work and a deliberate retry is fresh", async () => {
  resetHarness();
  const client = connectedClient();
  let resolveHistory;
  client.fetchEvents = () =>
    new Promise((resolve) => {
      resolveHistory = resolve;
    });
  const ipcCalls = [];
  const ipc = async (command) => {
    ipcCalls.push(command);
    return null;
  };
  const errors = [];
  const receiving = receiveLifecycle(
    { owner: "owner", community: "wss://one.example" },
    () => true,
    (error) => errors.push(error),
    ipc,
    client,
  );

  await flushUntil(() => sentProtocolFrames("REQ").length === 1);
  const retiredSubId = sentProtocolFrames("REQ")[0][1];
  await deliver(client, ["EOSE", retiredSubId]);
  await flushUntil(() => resolveHistory !== undefined);
  await deliver(client, [
    "EVENT",
    retiredSubId,
    { id: "queued-old", kind: 50182, created_at: 1 },
  ]);
  tickTo(20);
  await deliver(client, ["CLOSED", retiredSubId, "restricted: access revoked"]);
  resolveHistory([]);

  await assert.rejects(receiving, /receiver is unavailable/);
  await flushMicrotasks();
  assert.equal(
    ipcCalls.includes("receive_desktop_lifecycle"),
    false,
    "queued work from the retired receiver must not execute",
  );
  assert.match(errors.at(-1), /subscription closed/);

  client.fetchEvents = async () => [];
  const retried = receiveLifecycle(
    { owner: "owner", community: "wss://one.example" },
    () => true,
    (error) => errors.push(error),
    ipc,
    client,
  );
  await flushUntil(() => sentProtocolFrames("REQ").length === 2);
  const freshSubId = sentProtocolFrames("REQ")[1][1];
  assert.notEqual(freshSubId, retiredSubId);
  await deliver(client, ["EOSE", freshSubId]);
  const close = await retried;

  await deliver(client, [
    "EVENT",
    freshSubId,
    { id: "new-live", kind: 50182, created_at: 2 },
  ]);
  tickTo(40);
  await flushUntil(() => ipcCalls.includes("receive_desktop_lifecycle"));
  assert.equal(
    ipcCalls.filter((command) => command === "receive_desktop_lifecycle")
      .length,
    1,
  );
  await close();
});
