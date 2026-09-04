// ReadOnlyRelayClient publishes to inactive communities, but it shares the
// process-wide relay rate-limit gate with the primary session. Addressed EVENT
// refusals therefore need to settle this client's pending publish, arm the
// shared gate, and defer later sends until the advertised window expires.
import assert from "node:assert/strict";
import test from "node:test";

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;
const sends = [];

globalThis.window = {
  setTimeout: (fn, ms) => {
    const id = nextTimerId++;
    pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
    return id;
  },
  clearTimeout: (id) => pendingTimers.delete(id),
  __TAURI_INTERNALS__: {
    invoke: async (command, args) => {
      if (command === "plugin:websocket|send") sends.push(args);
    },
  },
};
Date.now = () => fakeNow;

const { ReadOnlyRelayClient } = await import("./readOnlyRelayClient.ts");
const { activateRateLimit, isRateLimited, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
}

function reset() {
  resetRateLimitGate();
  fakeNow = 0;
  pendingTimers.clear();
  nextTimerId = 1;
  sends.length = 0;
}

function connectedClient() {
  const client = new ReadOnlyRelayClient("wss://inactive.example");
  client.wsId = 7;
  client.connect = async () => {};
  return client;
}

function armPendingPublish(client, eventId) {
  const settled = new Promise((resolve, reject) => {
    client.publishes.set(eventId, {
      resolve,
      reject,
      timeout: window.setTimeout(() => {}, 25_000),
    });
  });
  return settled.then(
    () => ({ status: "resolved" }),
    (error) => ({ status: "rejected", error }),
  );
}

function deliver(client, frame) {
  return client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.generation,
  );
}

test("a rate-limited OK rejects the named publish and arms the shared gate", async () => {
  reset();
  const client = connectedClient();
  const eventId = "a".repeat(64);
  const settled = armPendingPublish(client, eventId);

  await deliver(client, [
    "OK",
    eventId,
    false,
    "rate-limited: quota exceeded; retry in 4s",
  ]);

  const outcome = await settled;
  assert.equal(outcome.status, "rejected");
  assert.match(outcome.error.message, /rate-limited/);
  assert.equal(client.publishes.has(eventId), false);
  assert.equal(isRateLimited(), true);
});

test("an ordinary OK rejection does not arm the shared gate", async () => {
  reset();
  const client = connectedClient();
  const eventId = "b".repeat(64);
  const settled = armPendingPublish(client, eventId);

  await deliver(client, ["OK", eventId, false, "invalid: bad signature"]);

  assert.equal((await settled).status, "rejected");
  assert.equal(isRateLimited(), false);
});

test("publish waits outside its timeout and pending state, then sends and settles", async () => {
  reset();
  activateRateLimit(4);
  const client = connectedClient();
  const event = { id: "c".repeat(64), kind: 5 };

  const published = client.publishEvent(event);
  await Promise.resolve();
  await Promise.resolve();

  assert.equal(sends.length, 0, "EVENT must remain unsent while gated");
  assert.equal(
    client.publishes.has(event.id),
    false,
    "publish timeout and pending ownership start only after the gate expires",
  );
  assert.equal(pendingTimers.size, 1, "only the gate timer should be armed");

  tickTo(4_000);
  await Promise.resolve();
  await Promise.resolve();

  assert.equal(sends.length, 1);
  assert.deepEqual(JSON.parse(sends[0].message.data), ["EVENT", event]);
  assert.equal(client.publishes.has(event.id), true);

  await deliver(client, ["OK", event.id, true, ""]);
  await published;
  assert.equal(client.publishes.has(event.id), false);
});

test("a disconnected client does not send after the shared gate expires", async () => {
  reset();
  activateRateLimit(4);
  const client = connectedClient();
  const event = { id: "d".repeat(64), kind: 5 };

  const published = client.publishEvent(event);
  await Promise.resolve();
  client.disconnect();
  tickTo(4_000);

  await assert.rejects(published, /not connected/);
  assert.equal(sends.length, 0);
  assert.equal(client.publishes.has(event.id), false);
});
