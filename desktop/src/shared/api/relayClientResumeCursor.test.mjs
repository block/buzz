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
const dials = [];
const repairRequests = [];
const clients = [];
let nextCallbackId = 1;
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
    transformCallback: () => nextCallbackId++,
    async invoke(command, args) {
      if (command === "plugin:websocket|connect") {
        dials.push(now);
        return 100 + dials.length;
      }
      if (command === "get_channel_reconnect_repair") {
        // The HTTP repair stays in flight until the test settles it.
        return new Promise((resolve) => repairRequests.push({ args, resolve }));
      }
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
const { buildChannelFilter } = await import("./relayChannelFilters.ts");

beforeEach(() => {
  resetRateLimitGate();
  now = 0;
  timers.clear();
  intervals.clear();
  writes.length = 0;
  dials.length = 0;
  repairRequests.length = 0;
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
  return { client, ids, ready };
}

const quiet = {
  kinds: [9, 5, 7],
  "#h": ["channel-0"],
  since: 123,
  limit: 1000,
};

test("a close after EOSE stamps liveUntil at the last frame and the replay resumes from it", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  const subscription = client.subscriptions.get(ids[0]);
  assert.equal(subscription.liveUntil, undefined);

  // EOSE (the last frame) arrived at 2_000_250; the close is processed later.
  now = 2_050_000;
  await close(client, 1000);
  assert.equal(subscription.liveUntil, 2_000);
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  // Reconnect late enough that liveUntil, not the silent-drop window, binds.
  now = 5_000_000;
  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  assert.deepEqual(
    writes.map(({ frame }) => frame),
    [["REQ", ids[0], { ...quiet, since: 2_000 - 960, limit: 1000 }]],
  );
});

test("a quiet subscription whose fan-out was dropped still replays the drop on a prompt reconnect", async () => {
  // The relay can drop a fan-out frame under backpressure and keep the socket
  // open, so pings after the drop do not prove the subscription missed nothing.
  now = 10_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.stallWatchdog.start();
  const droppedAt = 10_100; // stored, but its fan-out frame never arrived
  for (now = 10_030_000; now <= 12_000_000; now += 30_000) {
    await client.handleWsMessage(
      { type: "Ping", data: [] },
      client.connectionGeneration,
    );
  }
  now -= 30_000;
  await close(client, 1006);
  const subscription = client.subscriptions.get(ids[0]);
  assert.equal(subscription.liveUntil, Math.floor(now / 1_000));
  assert.ok(
    subscription.liveUntil - 960 > droppedAt,
    "precondition: the lookback alone would skip the drop",
  );
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  now += 2_000;
  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  const { since } = writes[0].frame[2];
  assert.equal(since, Math.floor(now / 1_000) - 3_600);
  assert.ok(
    droppedAt >= since,
    `drop at ${droppedAt} must satisfy since ${since}`,
  );
});

test("a subscription that never reached EOSE keeps replaying its original filter", async () => {
  now = 2_000_000;
  const second = { ...quiet, "#h": ["channel-1"] };
  const { client, ids } = await liveClient([quiet, second], 1);
  now = 2_050_000;
  await close(client, 1000);
  assert.equal(client.subscriptions.get(ids[0]).liveUntil, 2_000);
  assert.equal(client.subscriptions.get(ids[1]).liveUntil, undefined);
  window.clearTimeout(client.reconnectTimeout);
  client.reconnectTimeout = null;

  now = 5_000_000;
  client.wsId = 8;
  writes.length = 0;
  await client.replayLiveSubscriptions();
  const byId = new Map(writes.map(({ frame }) => [frame[1], frame[2]]));
  assert.equal(byId.get(ids[0]).since, 2_000 - 960);
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

// After a long pause (timers suspended) the native layer's terminal notice can
// be dequeued before the overdue idle tick. It is not proof the socket was
// alive, so it must not refresh the liveness clock that bounds `liveUntil`.
for (const [name, terminal] of [
  ["Close 1006", { type: "Close", data: { code: 1006, reason: "abnormal" } }],
  ["Close without a frame", { type: "Close", data: null }],
  ["Error", { type: "Error", data: "connection reset" }],
]) {
  test(`${name} after a long pause does not count as liveness`, async () => {
    now = 2_000_000;
    const { client, ids } = await liveClient([quiet]);
    client.stallWatchdog.start();
    now = 2_030_000;
    await deliver(client, ["NOTICE", "hello"]); // last real relay frame
    now = 2_030_000 + 3_600_000; // an hour asleep; the idle tick has not run
    await client.handleWsMessage(terminal, client.connectionGeneration);
    assert.equal(client.subscriptions.get(ids[0]).liveUntil, 2_030);
    window.clearTimeout(client.reconnectTimeout);
    client.reconnectTimeout = null;

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
}

test("a peer Ping is liveness; the close after it is not", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.stallWatchdog.start();
  now = 2_060_000;
  await client.handleWsMessage(
    { type: "Ping", data: [] },
    client.connectionGeneration,
  );
  now = 2_060_000 + 3_600_000;
  await close(client, 1006);
  assert.equal(client.subscriptions.get(ids[0]).liveUntil, 2_060);
});

test("a frame queued through a sleep does not count as liveness", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.stallWatchdog.start();
  now = 2_030_000;
  await deliver(client, ["NOTICE", "hello"]); // last frame before the sleep
  // A ping and a NOTICE were queued just before the sleep; the renderer
  // processes them on wake, ahead of the overdue idle tick and the close.
  now = 2_030_000 + 3_600_000;
  await client.handleWsMessage(
    { type: "Ping", data: [] },
    client.connectionGeneration,
  );
  now += 1_000;
  await deliver(client, ["NOTICE", "queued"]);
  await close(client, 1006);
  assert.equal(
    client.subscriptions.get(ids[0]).liveUntil,
    2_030,
    "stamped at the last frame before the gap, not at wake",
  );
});

test("a new connection's liveness credit does not inherit the previous socket's", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet]);
  client.relayUrl = "ws://relay.test";
  await close(client, 1006);
  // Reconnect well past the gap limit: the AUTH OK starts fresh credit
  // instead of reading as a frame after a gap on the old socket's clock.
  timers.get(client.reconnectTimeout).at += 300_000;
  await redial(client);
  const authAt = now;
  now += 30_000;
  await deliver(client, ["EOSE", ids[0]]);
  now += 1_000;
  await close(client, 1006);
  assert.equal(
    client.subscriptions.get(ids[0]).liveUntil,
    Math.floor((authAt + 30_000) / 1_000),
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

function fireDueTimers() {
  for (const [id, timer] of [...timers]) {
    if (timer.at <= now) {
      timers.delete(id);
      timer.fn();
    }
  }
}

// Drive the production reconnect: the scheduled timer dials through IPC and
// the relay's AUTH OK settles the handshake; replay then runs for real.
async function redial(client) {
  now = timers.get(client.reconnectTimeout).at;
  fireDueTimers();
  await flush();
  assert.ok(client.authRequest, "the redial is waiting for AUTH");
  await deliver(client, ["OK", client.authRequest.pendingEventId, true, ""]);
  await flush();
}

const paged = buildChannelFilter("channel-paged", 1000);

test("a reset before the paged live EOSE releases connect() and the next reconnect dials on time", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([paged]);
  client.relayUrl = "ws://relay.test";
  await close(client, 1006); // stamps liveUntil from the EOSE frame
  await redial(client);
  assert.equal(dials.length, 1);
  assert.deepEqual(writes.at(-1).frame, [
    "REQ",
    ids[0],
    { ...paged, limit: 0 },
  ]);
  assert.notEqual(client.connectPromise, null, "precondition: parked on EOSE");

  // The socket drops before the live-only REQ is acknowledged.
  await close(client, 1006);
  await flush();
  assert.equal(
    client.connectPromise,
    null,
    "the stale attempt settled at once",
  );
  assert.equal(intervals.size, 0, "no watchdog for a dead generation");
  assert.equal(repairRequests.length, 0);
  const retryAt = timers.get(client.reconnectTimeout).at;
  assert.ok(
    retryAt - now < 30_000,
    "the retry is a backoff, not the ack timeout",
  );

  await redial(client);
  assert.equal(dials.length, 2, "the scheduled reconnect opened a new socket");
  assert.equal(dials[1], retryAt);
});

test("a close delivered late during replay stamps the last frame, not the wake", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([quiet, paged]);
  client.relayUrl = "ws://relay.test";
  await close(client, 1006);
  await redial(client);
  // The quiet channel reaches EOSE while the paged repair is still pending.
  now += 30_000;
  const eoseAt = Math.floor(now / 1_000);
  await deliver(client, ["EOSE", ids[0]]);
  await deliver(client, ["EOSE", ids[1]]);
  await flush();
  assert.equal(repairRequests.length, 1, "precondition: repair in flight");
  assert.equal(intervals.size, 1, "the watchdog runs during replay");

  // An hour of sleep, then the close is delivered.
  now += 3_600_000;
  await close(client, 1006);
  const subscription = client.subscriptions.get(ids[0]);
  assert.equal(subscription.liveUntil, eoseAt, "stamped at the EOSE frame");

  // The in-flight HTTP page settles; the stale attempt then stands down.
  for (const { resolve } of repairRequests.splice(0)) resolve([]);
  await flush();
  assert.equal(client.connectPromise, null);
  writes.length = 0;
  await redial(client);
  const quietReq = writes.find(({ frame }) => frame[1] === ids[0]);
  assert.equal(quietReq.frame[2].since, eoseAt - 960);
});

test("a half-open socket that never acknowledges replay is reset by the stall watchdog", async () => {
  now = 2_000_000;
  const { client, ids } = await liveClient([paged]);
  client.relayUrl = "ws://relay.test";
  await close(client, 1006);
  await redial(client);
  assert.deepEqual(writes.at(-1).frame, [
    "REQ",
    ids[0],
    { ...paged, limit: 0 },
  ]);
  assert.notEqual(client.connectPromise, null, "precondition: parked on EOSE");
  assert.equal(client.connectionStateEmitter.get(), "connected");
  assert.equal(intervals.size, 1, "the watchdog is armed while parked");

  // No EOSE, no close: the idle tick is what notices.
  now += 70_000;
  const [tick] = intervals.values();
  tick();
  await flush();
  assert.equal(client.connectionStateEmitter.get(), "stalled");
  assert.equal(client.connectPromise, null, "connect() released by the reset");
  assert.equal(intervals.size, 0);
  assert.equal(repairRequests.length, 0, "no blind repair");
});

test("disposing a subscription parked on its paged live EOSE releases connect()", async () => {
  now = 2_000_000;
  const { client, ready } = await liveClient([paged]);
  const dispose = await ready[0];
  client.relayUrl = "ws://relay.test";
  await close(client, 1006);
  await redial(client);
  assert.notEqual(client.connectPromise, null, "precondition: parked on EOSE");
  await dispose();
  await flush();
  assert.equal(
    client.connectPromise,
    null,
    "connect() settled without the 25s wait",
  );
  assert.equal(client.connectionStateEmitter.get(), "connected");
});
