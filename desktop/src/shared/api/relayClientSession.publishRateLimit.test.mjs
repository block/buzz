import assert from "node:assert/strict";
import test from "node:test";

// ── Fake-timer setup ──────────────────────────────────────────────────────────
// The rate-limit gate and publish timeout both use window.setTimeout.

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;

function fakeSetTimeout(fn, ms) {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
  return id;
}

function fakeClearTimeout(id) {
  pendingTimers.delete(id);
}

async function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
  // Let promise chains hanging off fired timers (the gate) run.
  await new Promise((resolve) => setImmediate(resolve));
}

Date.now = () => fakeNow;
globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
  // resetConnection() closes the socket through the Tauri bridge; answer it
  // so the test stays quiet and a real warning cannot hide in the noise.
  __TAURI_INTERNALS__: {
    invoke: async (command) => {
      if (command === "plugin:websocket|disconnect") return;
      throw new Error(`Unexpected Tauri command: ${command}`);
    },
  },
};

// Import after the window shim is installed.
const { resetRateLimitGate, isRateLimited } = await import(
  "./relayRateLimitGate.ts"
);
const { RelayClient } = await import("./relayClientSession.ts");

const RATE_LIMITED = "rate-limited: quota exceeded; retry in 3s";

function makeClient() {
  pendingTimers.clear();
  nextTimerId = 1;
  fakeNow = 0;
  resetRateLimitGate();

  const client = new RelayClient();
  const sent = [];
  // Pretend the socket is open; capture frames instead of invoking Tauri.
  client.wsId = 1;
  client.sendRaw = async (payload) => {
    sent.push(payload);
  };
  return { client, sent };
}

function deliver(client, frame) {
  return client.handleWsMessage(JSON.stringify(frame), 0);
}

function watch(promise) {
  const state = { settled: false, value: undefined, error: undefined };
  promise.then(
    (value) => Object.assign(state, { settled: true, value }),
    (error) => Object.assign(state, { settled: true, error }),
  );
  return state;
}

const event = { id: "e1", kind: 9, content: "hi" };

test("OK=false rate-limited arms the gate and re-sends the EVENT once it clears", async () => {
  const { client, sent } = makeClient();
  const publish = watch(client.publishEvent(event, "timeout", "send failed"));
  await tickTo(0);
  assert.equal(sent.length, 1);

  await deliver(client, ["OK", "e1", false, RATE_LIMITED]);
  await tickTo(0);
  assert.equal(publish.settled, false, "first refusal must not fail the send");
  assert.equal(isRateLimited(), true, "gate armed from the OK reason");
  assert.equal(sent.length, 1, "no resend while the gate is closed");

  await tickTo(3_000);
  assert.equal(sent.length, 2, "EVENT re-sent when the gate cleared");
  assert.deepEqual(sent[1], ["EVENT", event]);

  await deliver(client, ["OK", "e1", true, ""]);
  await tickTo(3_000);
  assert.equal(
    publish.value,
    event,
    "retry's OK resolves the original publish",
  );
});

test("a second rate-limited refusal fails fast with the relay's reason", async () => {
  const { client, sent } = makeClient();
  const publish = watch(client.publishEvent(event, "timeout", "send failed"));
  await tickTo(0);

  await deliver(client, ["OK", "e1", false, RATE_LIMITED]);
  await tickTo(3_000);
  assert.equal(sent.length, 2);

  await deliver(client, ["OK", "e1", false, RATE_LIMITED]);
  await tickTo(3_000);
  assert.equal(publish.error?.message, RATE_LIMITED);
  assert.equal(sent.length, 2, "retry is bounded to one");
});

test("a non-rate-limit OK=false still rejects immediately", async () => {
  const { client } = makeClient();
  const publish = watch(client.publishEvent(event, "timeout", "send failed"));
  await tickTo(0);

  await deliver(client, [
    "OK",
    "e1",
    false,
    "restricted: not a channel member",
  ]);
  await tickTo(0);
  assert.equal(publish.error?.message, "restricted: not a channel member");
});

test("an uncorrelated NOTICE rate-limited arms the gate but never re-sends", async () => {
  const { client, sent } = makeClient();
  const publish = watch(client.publishEvent(event, "timeout", "send failed"));
  await tickTo(0);

  // The NOTICE may have been triggered by a REQ or COUNT, and an ephemeral
  // event the relay already fanned out would be delivered twice on resend.
  await deliver(client, ["NOTICE", RATE_LIMITED]);
  await tickTo(0);
  assert.equal(isRateLimited(), true, "gate armed from the NOTICE");
  assert.equal(publish.settled, false, "NOTICE cannot be attributed; no fail");

  await tickTo(3_000);
  assert.equal(sent.length, 1, "no resend from an uncorrelated NOTICE");

  await deliver(client, ["OK", "e1", true, ""]);
  await tickTo(3_000);
  assert.equal(publish.value, event);
});

test("a retry scheduled before a connection reset does not fire on the dead socket", async () => {
  const { client, sent } = makeClient();
  const publish = watch(client.publishEvent(event, "timeout", "send failed"));
  await tickTo(0);

  await deliver(client, ["OK", "e1", false, RATE_LIMITED]);
  await tickTo(0);
  client.resetConnection(new Error("Relay connection closed."), {
    reconnect: false,
  });
  await tickTo(0);
  assert.equal(publish.error?.message, "Relay connection closed.");

  await tickTo(3_000);
  assert.equal(sent.length, 1, "no resend after the publish was settled");
});
