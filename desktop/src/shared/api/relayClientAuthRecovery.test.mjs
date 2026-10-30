import assert from "node:assert/strict";
import test from "node:test";

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;

globalThis.window = {
  setTimeout: (fn, delayMs) => {
    const id = nextTimerId++;
    pendingTimers.set(id, { fn, fireAt: fakeNow + delayMs, delayMs });
    return id;
  },
  clearTimeout: (id) => pendingTimers.delete(id),
  __TAURI_INTERNALS__: {
    invoke: async () => undefined,
  },
};

const originalRandom = Math.random;
Math.random = () => 0.999;

const { RelayClient } = await import("./relayClientSession.ts");
const { armRelayAuthentication } = await import("./relayAuthPolicy.ts");
const { AUTH_TIMEOUT_MS, RECONNECT_BASE_DELAY_MS, RECONNECT_MAX_DELAY_MS } =
  await import("./relayClientTimings.ts");

function resetHarness() {
  fakeNow = 0;
  pendingTimers.clear();
  nextTimerId = 1;
  Math.random = () => 0.999;
}

function connectedClient() {
  const client = new RelayClient();
  client.wsId = 7;
  client.connectionGeneration = 1;
  client.keepAliveRequested = true;
  client.hasConnectedOnce = true;
  client.connectionStateEmitter.set("connected");
  return client;
}

function armAuthRequest(client, attempt) {
  const authentication = armRelayAuthentication(
    AUTH_TIMEOUT_MS,
    (request) => {
      request.pendingEventId = `auth-${attempt}`;
      client.authRequest = request;
    },
    (error) => client.resetConnection(error),
  );
  authentication.catch(() => {});
  return authentication;
}

function installAutomaticReconnect(client) {
  let attempts = 0;
  client.ensureConnected = () => {
    attempts++;
    client.connectionGeneration++;
    client.wsId = 7 + attempts;
    const authentication = armAuthRequest(client, attempts);
    return authentication.then(() => {
      client.connectionStateEmitter.set("connected");
      return client.connectionGeneration;
    });
  };
  return () => attempts;
}

async function deliver(
  client,
  frame,
  generation = client.connectionGeneration,
) {
  await client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    generation,
  );
}

async function flushMicrotasks() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

async function fireReconnect(client) {
  const timerId = client.reconnectTimeout;
  assert.notEqual(timerId, null, "an automatic reconnect should be scheduled");
  const timer = pendingTimers.get(timerId);
  assert.ok(timer, "the scheduled reconnect should have a timer");
  pendingTimers.delete(timerId);
  fakeNow = Math.max(fakeNow, timer.fireAt);
  timer.fn();
  await flushMicrotasks();
  assert.ok(client.authRequest, "the automatic attempt should reach AUTH");
}

test("server-unavailable AUTH replies keep retrying through capped backoff and recover", async () => {
  resetHarness();
  const client = connectedClient();
  const automaticReconnects = installAutomaticReconnect(client);
  const unavailableReplies = [
    "error: internal error checking restriction state",
    "error: internal error checking allowlist",
    "error: internal error checking relay membership",
    "error: internal error recording agent owner",
  ];
  const delays = [];
  let expectedBase = RECONNECT_BASE_DELAY_MS;
  armAuthRequest(client, 0);

  for (let attempt = 0; attempt < 8; attempt++) {
    const generation = client.connectionGeneration;
    const eventId = client.authRequest.pendingEventId;
    await deliver(client, [
      "OK",
      eventId,
      false,
      unavailableReplies[attempt % unavailableReplies.length],
    ]);

    assert.equal(client.terminal, false);
    assert.equal(client.getConnectionState(), "reconnecting");
    const timer = pendingTimers.get(client.reconnectTimeout);
    assert.ok(timer, "a server-unavailable AUTH reply must schedule a retry");
    const expectedDelay = Math.min(
      expectedBase * (0.75 + 0.999 * 0.5),
      RECONNECT_MAX_DELAY_MS,
    );
    assert.equal(timer.delayMs, expectedDelay);
    assert.ok(timer.delayMs >= expectedBase * 0.75);
    assert.ok(timer.delayMs <= RECONNECT_MAX_DELAY_MS);
    delays.push(timer.delayMs);
    expectedBase = Math.min(expectedBase * 2, RECONNECT_MAX_DELAY_MS);

    if (attempt === 0) {
      const scheduledTimer = client.reconnectTimeout;
      await deliver(
        client,
        ["OK", eventId, false, "blocked: you are banned from this community"],
        generation,
      );
      assert.equal(client.terminal, false, "stale AUTH frames are ignored");
      assert.equal(client.reconnectTimeout, scheduledTimer);
    }

    await fireReconnect(client);
  }

  assert.deepEqual(delays.slice(5), [30_000, 30_000, 30_000]);
  assert.equal(automaticReconnects(), 8);
  await deliver(client, ["OK", client.authRequest.pendingEventId, true, ""]);
  await flushMicrotasks();

  assert.equal(client.getConnectionState(), "connected");
  assert.equal(client.terminal, false);
  assert.notEqual(client.wsId, null);
  assert.equal(client.reconnectTimeout, null);
});

test("bans and nonmembership remain terminal after a transient writer failure", async () => {
  for (const message of [
    "blocked: you are banned from this community",
    "restricted: not a relay member",
  ]) {
    resetHarness();
    const client = connectedClient();
    installAutomaticReconnect(client);
    armAuthRequest(client, 0);

    await deliver(client, [
      "OK",
      client.authRequest.pendingEventId,
      false,
      "error: internal error checking relay membership",
    ]);
    await fireReconnect(client);
    await deliver(client, [
      "OK",
      client.authRequest.pendingEventId,
      false,
      message,
    ]);

    assert.equal(client.terminal, true);
    assert.equal(client.getConnectionState(), "disconnected");
    assert.equal(client.reconnectTimeout, null);
  }
});

test("invalid and unknown AUTH evidence still hits the three-rejection cap", async () => {
  for (const messages of [
    [
      "auth-required: verification failed",
      "auth-required: verification failed",
      "auth-required: verification failed",
    ],
    ["", "unrecognized AUTH failure", "error: internal"],
  ]) {
    resetHarness();
    const client = connectedClient();
    installAutomaticReconnect(client);
    armAuthRequest(client, 0);

    for (let index = 0; index < messages.length; index++) {
      await deliver(client, [
        "OK",
        client.authRequest.pendingEventId,
        false,
        messages[index],
      ]);
      assert.equal(client.terminal, index === messages.length - 1);
      if (index < messages.length - 1) await fireReconnect(client);
    }
    assert.equal(client.reconnectTimeout, null);
  }
});

test("unavailable replies do not spend or erase an invalid-evidence streak", async () => {
  resetHarness();
  const client = connectedClient();
  installAutomaticReconnect(client);
  armAuthRequest(client, 0);

  for (let index = 0; index < 2; index++) {
    await deliver(client, [
      "OK",
      client.authRequest.pendingEventId,
      false,
      "auth-required: verification failed",
    ]);
    assert.equal(client.terminal, false);
    await fireReconnect(client);
  }

  await deliver(client, [
    "OK",
    client.authRequest.pendingEventId,
    false,
    "error: internal error checking relay membership",
  ]);
  assert.equal(client.terminal, false);
  await fireReconnect(client);

  await deliver(client, [
    "OK",
    client.authRequest.pendingEventId,
    false,
    "unrecognized AUTH failure",
  ]);
  assert.equal(client.terminal, true);
  assert.equal(client.reconnectTimeout, null);
});

test.after(() => {
  Math.random = originalRandom;
});
