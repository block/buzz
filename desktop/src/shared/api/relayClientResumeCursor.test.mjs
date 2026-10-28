// Exercise the production session: EOSE -> socket close -> resume cursor ->
// replayed REQ, and the close-code-driven reconnect spreading.
// The IPC transport and clock are fake; no native app or external relay is used.
import assert from "node:assert/strict";
import { after, beforeEach, afterEach, test } from "node:test";

const originalNow = Date.now;
const originalRandom = Math.random;
const originalWindow = globalThis.window;
let now = 0;
let nextTimerId = 1;
const timers = new Map();
const intervals = new Map();
const writes = [];
const clients = [];
globalThis.window = {
  setTimeout(fn, ms) {
    const id = nextTimerId++;
    timers.set(id, { fn, at: now + ms });
    return id;
  },
  clearTimeout(id) {
    timers.delete(id);
  },
  // The stall watchdog's idle check; fired explicitly by the stall test.
  setInterval(fn) {
    const id = nextTimerId++;
    intervals.set(id, fn);
    return id;
  },
  clearInterval(id) {
    intervals.delete(id);
  },
  __TAURI_INTERNALS__: {
    async invoke(command, args) {
      if (command === "plugin:websocket|send") {
        writes.push({ socket: args.id, frame: JSON.parse(args.message.data) });
        return;
      }
      if (command === "plugin:websocket|disconnect") return;
      assert.fail(`unexpected IPC: ${command}`);
    },
  },
};
Date.now = () => now;
const { RelayClient } = await import("./relayClientSession.ts");
const { resetRateLimitGate } = await import("./relayRateLimitGate.ts");

beforeEach(() => {
  resetRateLimitGate();
  now = 0;
  timers.clear();
  intervals.clear();
  writes.length = 0;
  Math.random = originalRandom;
});
afterEach(() => {
  for (const client of clients.splice(0)) client.disconnect();
  resetRateLimitGate();
  Math.random = originalRandom;
  assert.equal(timers.size, 0, "session must release its timers");
});
after(() => {
  Date.now = originalNow;
  Math.random = originalRandom;
  globalThis.window = originalWindow;
});

async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}
function deliver(client, frame) {
  return client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.connectionGeneration,
  );
}
function close(client, code) {
  return client.handleWsMessage(
    { type: "Close", data: { code, reason: "test" } },
    client.connectionGeneration,
  );
}
function reconnectDelayMs(client) {
  const timer = timers.get(client.reconnectTimeout);
  assert.ok(timer, "a reconnect must be scheduled");
  return timer.at - now;
}

// A client with one live, non-paged subscription per filter. Only the first
// `eosed` of them receive EOSE on the first connection.
async function liveClient(filters, eosed = filters.length) {
  const client = new RelayClient();
  client.wsId = 7; // Already authenticated; subscribe and dispatch stay real.
  clients.push(client);
  const ready = filters.map((filter) => client.subscribeLive(filter, () => {}));
  for (const pending of ready) void pending.catch(() => {});
  await flush();
  for (let i = 0; i < filters.length; i++) {
    now += 250;
    for (const [id, timer] of [...timers]) {
      if (timer.at <= now) {
        timers.delete(id);
        timer.fn();
      }
    }
    await flush();
  }
  const reqs = writes.filter(({ frame }) => frame[0] === "REQ");
  assert.equal(reqs.length, filters.length);
  for (const { frame } of reqs.slice(0, eosed)) {
    await deliver(client, ["EOSE", frame[1]]);
  }
  const ids = reqs.map(({ frame }) => frame[1]);
  // Resolve any outstanding readiness without leaving timers behind.
  for (const [id, timer] of [...timers]) {
    timers.delete(id);
    timer.fn();
  }
  await flush();
  return { client, ids };
}

const quiet = {
  kinds: [9, 5, 7],
  "#h": ["channel-0"],
  since: 123,
  limit: 1000,
};

test("a close after EOSE stamps liveUntil and the replay resumes from it", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  const subscription = client.subscriptions.get(ids[0]);
  assert.equal(subscription.liveUntil, undefined);

  now = 2_050_000;
  await close(client, 1000);
  assert.equal(subscription.liveUntil, 2_050);
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  now = 2_100_000;
  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  assert.deepEqual(
    writes.map(({ frame }) => frame),
    [["REQ", ids[0], { ...quiet, since: 2_050 - 960, limit: 1000 }]],
  );
});

test("a subscription that never reached EOSE keeps replaying its original filter", async () => {
  now = 2_000_000;
  const second = { ...quiet, "#h": ["channel-1"] };
  const { client, ids } = await liveClient([quiet, second], 1);
  now = 2_050_000;
  await close(client, 1000);
  assert.equal(client.subscriptions.get(ids[0]).liveUntil, 2_050);
  assert.equal(client.subscriptions.get(ids[1]).liveUntil, undefined);
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  const byId = new Map(writes.map(({ frame }) => [frame[1], frame[2]]));
  assert.equal(byId.get(ids[0]).since, 2_050 - 960);
  assert.deepEqual(byId.get(ids[1]), second);
});

test("a relay CLOSED after EOSE means the socket close must not stamp the subscription", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  await deliver(client, ["CLOSED", ids[0], "error: database error"]);
  now = 2_050_000;
  await close(client, 1000);
  assert.equal(client.subscriptions.get(ids[0]).liveUntil, undefined);
});

test("a stalled socket stamps liveUntil at its last inbound frame, not at detection", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.stallWatchdog.start(); // lastInboundAt = 2_000_000
  now = 2_030_000;
  await deliver(client, ["NOTICE", "hello"]); // recordInbound via handleWsMessage
  now = 2_100_000;
  client.resetConnection(new Error("Relay socket stalled"));
  assert.equal(client.subscriptions.get(ids[0]).liveUntil, 2_030);
});

test("the real stall path stamps liveUntil at the pre-sleep inbound frame", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.stallWatchdog.start();
  now = 2_030_000;
  await deliver(client, ["NOTICE", "hello"]); // last inbound frame
  // The machine sleeps for an hour; the first overdue idle tick is the wake.
  now = 2_030_000 + 3_600_000;
  assert.equal(intervals.size, 1, "the production watchdog interval is armed");
  const [tick] = intervals.values();
  tick(); // checkIdle -> stop() -> production onStall -> resetConnection
  const subscription = client.subscriptions.get(ids[0]);
  assert.equal(client.connectionStateEmitter.get(), "stalled");
  assert.equal(
    subscription.liveUntil,
    2_030,
    "stamped at the last inbound frame, not at wake time",
  );
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  // A message stored early in the gap must be inside the reconnect REQ.
  const earlyInGap = 2_030 + 120;
  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  const { since } = writes[0].frame[2];
  assert.ok(
    earlyInGap >= since,
    `message at ${earlyInGap} must satisfy REQ since ${since}`,
  );
});

for (const code of [1012, 1001]) {
  test(`close ${code} spreads the first reconnect over [0, 10s] and keeps the backoff escalated`, async () => {
    const { client } = await liveClient([quiet]);
    Math.random = () => 0.37;
    // A client bounced repeatedly is already backed off.
    for (let i = 0; i < 3; i++) client.reconnectBackoff.next();
    assert.equal(client.reconnectBackoff.currentMs, 8_000);
    await close(client, code);
    assert.equal(reconnectDelayMs(client), 3_700);
    assert.equal(client.reconnectBackoff.currentMs, 16_000);
  });
}

test("an ordinary close backs off with +/-25% jitter, not the restart spread", async () => {
  const { client } = await liveClient([quiet]);
  Math.random = () => 0.37;
  await close(client, 1000);
  assert.ok(Math.abs(reconnectDelayMs(client) - 935) < 1e-6);
  assert.equal(client.reconnectBackoff.currentMs, 2_000);
});
