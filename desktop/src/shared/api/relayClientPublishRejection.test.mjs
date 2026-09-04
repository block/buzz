// A relay rejection addressed to one event must settle that event's pending
// publish *and* arm the rate-limit gate.
//
// History: the relay rejected an over-quota EVENT with a bare
// `["NOTICE", "rate-limited: ..."]`. A NOTICE carries no event id, and
// `pendingEvents` is keyed by event id, so nothing settled — the publish sat
// until PUBLISH_TIMEOUT_MS (25s) and surfaced as a message stuck on
// "Sending…". Startup quota exhaustion made that routine in the first seconds
// after launch. The relay now rejects on the OK channel instead, so the gate
// arming that used to live in the NOTICE branch has to happen here too.
import assert from "node:assert/strict";
import test from "node:test";

const fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;
const sendAttempts = [];
const deliveredFrames = [];
let sendTransport = async (args) => {
  deliveredFrames.push(args);
};

globalThis.window = {
  setTimeout: (fn, ms) => {
    const id = nextTimerId++;
    pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
    return id;
  },
  clearTimeout: (id) => pendingTimers.delete(id),
  __TAURI_INTERNALS__: {
    invoke: async (command, args) => {
      if (command === "plugin:websocket|send") {
        sendAttempts.push(args);
        return sendTransport(args);
      }
    },
  },
};
Date.now = () => fakeNow;

const { RelayClient } = await import("./relayClientSession.ts");
const { activateRateLimit, isRateLimited, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

function reset() {
  resetRateLimitGate();
  pendingTimers.clear();
  nextTimerId = 1;
  sendAttempts.length = 0;
  deliveredFrames.length = 0;
  sendTransport = async (args) => {
    deliveredFrames.push(args);
  };
}

function connectedClient() {
  const client = new RelayClient();
  client.wsId = 7;
  return client;
}

function eventFrames() {
  return deliveredFrames.filter(
    ({ message }) => JSON.parse(message.data)[0] === "EVENT",
  );
}

async function flushUntil(predicate, attempts = 20) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail("condition did not become true before the microtask limit");
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

/**
 * Registers a pending publish the way `publishEvent` does, without needing a
 * socket: the OK dispatch under test only reads `pendingEvents`.
 */
function armPendingPublish(client, eventId) {
  const event = { id: eventId };
  const settled = new Promise((resolve, reject) => {
    client.pendingEvents.set(eventId, {
      event,
      resolve,
      reject,
      timeout: window.setTimeout(() => {}, 25_000),
    });
  });
  // Keep the rejection from surfacing as an unhandled rejection.
  return settled.then(
    (value) => ({ status: "resolved", value }),
    (error) => ({ status: "rejected", error }),
  );
}

/** Feeds a raw relay frame through the real inbound dispatch path. */
function deliver(client, frame) {
  return client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.connectionGeneration,
  );
}

test("a rate-limited OK rejection settles the pending publish", async () => {
  resetRateLimitGate();
  pendingTimers.clear();
  const client = new RelayClient();
  const eventId = "a".repeat(64);
  const settled = armPendingPublish(client, eventId);

  await deliver(client, [
    "OK",
    eventId,
    false,
    "rate-limited: quota exceeded; retry in 4s",
  ]);

  const outcome = await settled;
  assert.equal(
    outcome.status,
    "rejected",
    "an over-quota publish must fail fast, not hang until the 25s publish timeout",
  );
  assert.match(outcome.error.message, /rate-limited/);
  assert.equal(
    client.pendingEvents.has(eventId),
    false,
    "the pending entry must be cleared",
  );
});

test("a rate-limited OK rejection arms the rate-limit gate", async () => {
  resetRateLimitGate();
  pendingTimers.clear();
  const client = new RelayClient();
  const eventId = "b".repeat(64);
  const settled = armPendingPublish(client, eventId);

  assert.equal(isRateLimited(), false, "gate starts closed");

  await deliver(client, [
    "OK",
    eventId,
    false,
    "rate-limited: quota exceeded; retry in 4s",
  ]);
  await settled;

  assert.equal(
    isRateLimited(),
    true,
    "back-pressure now arrives on the OK channel — without arming here the " +
      "client fails the send and immediately retries into the same quota",
  );
});

test("an ordinary OK rejection does not arm the gate", async () => {
  resetRateLimitGate();
  pendingTimers.clear();
  const client = new RelayClient();
  const eventId = "c".repeat(64);
  const settled = armPendingPublish(client, eventId);

  await deliver(client, ["OK", eventId, false, "invalid: bad signature"]);
  const outcome = await settled;

  assert.equal(outcome.status, "rejected");
  assert.equal(
    isRateLimited(),
    false,
    "only `rate-limited:` rejections signal back-pressure",
  );
});

test("an accepted OK still resolves the pending publish", async () => {
  reset();
  const client = new RelayClient();
  const eventId = "d".repeat(64);
  const settled = armPendingPublish(client, eventId);

  await deliver(client, ["OK", eventId, true, ""]);
  const outcome = await settled;

  assert.equal(outcome.status, "resolved");
  assert.equal(outcome.value.id, eventId);
});

test("a publish started during an ordinary outage reconnects once and settles", async () => {
  reset();
  const client = new RelayClient();
  const event = { id: "0".repeat(64), kind: 1 };
  let reconnects = 0;
  client.ensureConnected = async () => {
    reconnects++;
    client.connectionGeneration++;
    client.wsId = 8;
    return client.connectionGeneration;
  };

  const published = client.publishEvent(event, "timed out", "send failed");
  await flushUntil(() => eventFrames().length === 1);

  assert.equal(reconnects, 1);
  assert.equal(sendAttempts.length, 1);
  assert.equal(client.pendingEvents.has(event.id), true);

  await deliver(client, ["OK", event.id, true, ""]);
  assert.equal(await published, event);
  assert.equal(client.pendingEvents.size, 0);
});

test("a community switch while gated cannot publish through its replacement socket", async () => {
  reset();
  activateRateLimit(4);
  const client = connectedClient();
  const event = { id: "e".repeat(64), kind: 1 };

  const published = client.publishEvent(event, "timed out", "send failed");
  await Promise.resolve();
  assert.equal(client.pendingEvents.size, 0);

  client.disconnect();
  resetRateLimitGate();
  client.wsId = 8;

  await assert.rejects(published, /community switch/);
  assert.equal(client.pendingEvents.size, 0);
  assert.equal(eventFrames().length, 0);
});

test("a community switch after send failure cannot retry through its replacement socket", async () => {
  reset();
  const client = connectedClient();
  const event = { id: "f".repeat(64), kind: 1 };
  const reconnect = deferred();
  client.ensureConnected = async () => {
    await reconnect.promise;
    return client.connectionGeneration;
  };
  sendTransport = async () => {
    throw new Error("old socket failed");
  };

  const published = client.publishEvent(event, "timed out", "send failed");
  const outcome = published.then(
    () => ({ status: "resolved" }),
    (error) => ({ status: "rejected", error }),
  );
  await flushUntil(() => client.connectionGeneration === 1);
  assert.equal(sendAttempts.length, 1);
  assert.equal(eventFrames().length, 0);
  assert.equal(
    client.connectionGeneration,
    1,
    "the failed send reset its socket",
  );
  assert.equal(
    client.pendingEvents.has(event.id),
    true,
    "the original publish remains owned while reconnect is pending",
  );

  client.disconnect();
  client.wsId = 8;
  const settledBeforeReconnect = await outcome;
  assert.equal(
    settledBeforeReconnect.status,
    "rejected",
    "community switch must settle the publish without waiting for reconnect",
  );
  assert.match(settledBeforeReconnect.error.message, /community switch/);

  reconnect.resolve();
  await published.catch(() => {});
  await Promise.resolve();
  assert.equal(client.pendingEvents.size, 0);
  assert.equal(
    sendAttempts.length,
    1,
    "the replacement socket must not be used",
  );
  assert.equal(eventFrames().length, 0);
});
