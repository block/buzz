// Exercise the production session from live registration/refusal through actual IPC dispatch.
// The IPC transport and clock are fake; no native app or external relay is used.
import assert from "node:assert/strict";
import { after, beforeEach, afterEach, test } from "node:test";

const originalNow = Date.now;
const originalWindow = globalThis.window;
let now = 0;
let nextTimerId = 1;
const timers = new Map();
const writes = [];
const clients = [];
let onSend;
globalThis.window = {
  setTimeout(fn, ms) {
    const id = nextTimerId++;
    timers.set(id, { fn, at: now + ms });
    return id;
  },
  clearTimeout(id) {
    timers.delete(id);
  },
  __TAURI_INTERNALS__: {
    async invoke(command, args) {
      if (command === "plugin:websocket|send") {
        writes.push({
          socket: args.id,
          frame: JSON.parse(args.message.data),
          at: now,
        });
        await onSend?.(writes.at(-1));
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

const { openPresenceSubscription } = await import(
  "./presenceRelaySubscription.ts"
);

beforeEach(() => {
  resetRateLimitGate();
  now = 0;
  timers.clear();
  writes.length = 0;
  onSend = undefined;
});
afterEach(() => {
  for (const client of clients.splice(0)) client.disconnect();
  resetRateLimitGate();
  assert.equal(
    timers.size,
    0,
    "subscriptions and gate must release their timers",
  );
});
after(() => {
  Date.now = originalNow;
  globalThis.window = originalWindow;
});

async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}
async function tickTo(target) {
  assert.ok(target >= now);
  let fired = 0;
  for (;;) {
    const next = [...timers]
      .filter(([, timer]) => timer.at <= target)
      .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
    if (!next) break;
    assert.ok(++fired < 1000, "timer loop must make progress");
    now = next[1].at;
    timers.delete(next[0]);
    next[1].fn();
    await flush();
  }
  now = target;
  await flush();
}
function deliver(client, frame) {
  return client.handleWsMessage(
    { type: "Text", data: JSON.stringify(frame) },
    client.connectionGeneration,
  );
}
function frames(op) {
  return writes.filter((write) => write.frame[0] === op);
}

function session() {
  const client = new RelayClient();
  client.wsId = 7;
  clients.push(client);
  return client;
}
function start(client, count) {
  return Array.from({ length: count }, (_, i) => {
    const filter = {
      kinds: [9, 5, 7],
      "#h": [`channel-${i}`],
      since: 123,
      limit: 1000,
    };
    const controller = new AbortController();
    const ready = [];
    const promise = client.subscribeLive(
      filter,
      () => {},
      (r) => ready.push(r),
      250,
      controller.signal,
    );
    void promise.catch(() => {});
    return { filter, controller, ready, promise };
  });
}

test("cold setup drains at most one live REQ per 250ms, prioritizes visible channel and preserves every filter", async () => {
  const client = session();
  const entries = start(client, 296);
  await flush();
  assert.equal(frames("REQ").length, 1, "startup must not dump 296 REQs");
  assert.deepEqual(
    entries[295].ready,
    [],
    "queued setup is not yet a readiness timeout",
  );
  client.setVisibleChannelId("channel-295");
  await tickTo(250);
  assert.deepEqual(frames("REQ")[1].frame[2], entries[295].filter);
  await tickTo(73750);
  assert.equal(frames("REQ").length, 296);
  const requests = frames("REQ");
  for (let i = 1; i < requests.length; i++)
    assert.ok(requests[i].at - requests[i - 1].at >= 250);
  assert.deepEqual(
    new Set(requests.map((r) => JSON.stringify(r.frame[2]))),
    new Set(entries.map((e) => JSON.stringify(e.filter))),
  );
  for (const { frame } of requests) await deliver(client, ["EOSE", frame[1]]);
  await Promise.all(entries.map((e) => e.promise));
});

test("125 refused live subscriptions cannot stampede beside a publish when cooldown releases", async () => {
  const client = session();
  const entries = start(client, 125);
  await flush();
  await tickTo(31250);
  const initial = frames("REQ");
  assert.equal(initial.length, 125);
  for (const { frame } of initial) await deliver(client, ["EOSE", frame[1]]);
  await Promise.all(entries.map((e) => e.promise));
  for (const { frame } of initial)
    await deliver(client, [
      "CLOSED",
      frame[1],
      "rate-limited: quota exceeded; retry in 4s",
    ]);
  const event = { id: "a".repeat(64), kind: 9 };
  const published = client.publishEvent(event, "timeout", "send failed");
  void published.catch(() => {});
  await tickTo(35250);
  assert.ok(
    frames("REQ").length <= 126,
    "only one retry may join Send at gate release",
  );
  assert.equal(
    frames("EVENT").length,
    1,
    "Send must not await the live backlog",
  );
  await deliver(client, ["OK", event.id, true, ""]);
  assert.equal(await published, event);
  await tickTo(66250);
  assert.equal(frames("REQ").length, 250);
  const retries = frames("REQ").slice(125);
  assert.deepEqual(
    retries.map((r) => r.frame.slice(1)),
    initial.map((r) => r.frame.slice(1)),
  );
  for (let i = 1; i < retries.length; i++)
    assert.ok(retries[i].at - retries[i - 1].at >= 250);
});

test("presence waits beyond its 5s budget behind the gate, then EOSE before IPC settles succeeds", async () => {
  const client = session();
  const background = start(client, 32);
  await flush();
  await deliver(client, [
    "NOTICE",
    "rate-limited: quota exceeded; retry in 6s",
  ]);
  let outcome = "pending";
  const presence = openPresenceSubscription(
    ["a".repeat(64)],
    () => {},
    client.subscribeLive.bind(client),
  );
  presence.then(
    () => {
      outcome = "ready";
    },
    () => {
      outcome = "failed";
    },
  );
  await flush();
  await tickTo(5999);
  assert.equal(outcome, "pending");
  const presenceEntry = [...client.subscriptions].find(
    ([, sub]) => sub.filter.limit === 0,
  );
  assert.ok(presenceEntry, "presence remains owned while queued");
  assert.equal(frames("REQ").length, 1);
  const sent = Promise.withResolvers();
  onSend = async ({ frame }) => {
    if (frame[0] !== "REQ" || frame[1] !== presenceEntry[0]) return;
    await deliver(client, ["EOSE", frame[1]]);
    await sent.promise;
  };
  try {
    await tickTo(6000);
    assert.equal(
      frames("REQ")[1].frame[1],
      presenceEntry[0],
      "limit:0 outranks the background backlog",
    );
    await tickTo(12000);
    assert.equal(outcome, "pending", "setup still awaits transport completion");
    sent.resolve();
    await flush();
    const dispose = await presence;
    assert.equal(
      outcome,
      "ready",
      "queued time must not consume presence readiness",
    );
    assert.ok(client.subscriptions.has(presenceEntry[0]));
    await dispose();
  } finally {
    sent.resolve();
    for (const entry of background) entry.controller.abort();
  }
});

test("a later gate extension pauses an active drain without changing filters or counting another retry", async () => {
  const client = session();
  const entries = start(client, 4);
  await flush();
  await tickTo(750);
  const initial = frames("REQ");
  for (const { frame } of initial) await deliver(client, ["EOSE", frame[1]]);
  await Promise.all(entries.map((e) => e.promise));
  for (const { frame } of initial)
    await deliver(client, ["CLOSED", frame[1], "error: temporary failure"]);
  await tickTo(1750);
  assert.equal(frames("REQ").length, 5);
  await tickTo(1800);
  await deliver(client, [
    "NOTICE",
    "rate-limited: quota exceeded; retry in 4s",
  ]);
  await tickTo(3000);
  await deliver(client, [
    "NOTICE",
    "rate-limited: quota exceeded; retry in 4s",
  ]);
  await tickTo(6999);
  assert.equal(frames("REQ").length, 5);
  for (const { frame } of initial)
    assert.equal(client.subscriptions.get(frame[1]).closedRetryAttempt, 1);
  await tickTo(7500);
  assert.deepEqual(
    frames("REQ")
      .slice(4)
      .map((r) => r.at),
    [1750, 7000, 7250, 7500],
  );
  assert.deepEqual(
    frames("REQ")
      .slice(4)
      .map((r) => r.frame),
    initial.map((r) => r.frame),
  );
});

for (const stop of ["abort", "terminal CLOSED", "workspace switch"]) {
  test(`${stop} retires queued cold setup without a later REQ`, async () => {
    const client = session();
    const entries = start(client, 2);
    await flush();
    const [id] = [...client.subscriptions].find(
      ([, sub]) => sub.filter["#h"][0] === "channel-1",
    );
    if (stop === "abort") entries[1].controller.abort();
    if (stop === "terminal CLOSED")
      await deliver(client, ["CLOSED", id, "restricted: denied"]);
    if (stop === "workspace switch") {
      client.disconnect();
      client.wsId = 8;
    }
    const result = await Promise.allSettled([entries[1].promise]);
    assert.equal(
      result[0].status,
      stop === "terminal CLOSED" ? "fulfilled" : "rejected",
    );
    assert.ok(!client.subscriptions.has(id));
    await tickTo(60000);
    assert.equal(frames("REQ").length, 1);
    assert.ok(!writes.some((w) => w.socket === 8));
  });
}

for (const stop of [
  "dispose",
  "EOSE",
  "EVENT",
  "terminal CLOSED",
  "workspace switch",
]) {
  test(`${stop} cancels a retry already queued in the drain`, async () => {
    const client = session();
    const entries = start(client, 2);
    await flush();
    await tickTo(250);
    const initial = frames("REQ");
    for (const { frame } of initial) await deliver(client, ["EOSE", frame[1]]);
    const disposers = await Promise.all(entries.map((e) => e.promise));
    for (const { frame } of initial)
      await deliver(client, ["CLOSED", frame[1], "error: temporary failure"]);
    await tickTo(1250);
    assert.equal(frames("REQ").length, 3);
    const id = initial[1].frame[1];
    if (stop === "dispose") await disposers[1]();
    if (stop === "EOSE") await deliver(client, ["EOSE", id]);
    if (stop === "EVENT")
      await deliver(client, [
        "EVENT",
        id,
        { id: "recovered", kind: 9, created_at: 123 },
      ]);
    if (stop === "terminal CLOSED")
      await deliver(client, ["CLOSED", id, "restricted: denied"]);
    if (stop === "workspace switch") {
      client.disconnect();
      client.wsId = 8;
    }
    await tickTo(60000);
    assert.equal(frames("REQ").length, 3);
    assert.ok(!writes.some((w) => w.socket === 8));
  });
}

for (const stage of ["setup", "CLOSED retry"]) {
  test(`reconnect takes over queued ${stage} without a second drain dispatch`, async () => {
    const client = session();
    const entries = start(client, 3);
    await flush();
    if (stage === "CLOSED retry") {
      await tickTo(500);
      for (const { frame } of frames("REQ"))
        await deliver(client, ["EOSE", frame[1]]);
      await Promise.all(entries.map((e) => e.promise));
      for (const { frame } of frames("REQ"))
        await deliver(client, ["CLOSED", frame[1], "error: temporary failure"]);
      await tickTo(1500);
      assert.equal(frames("REQ").length, 4);
    }
    const originalIds = [...client.subscriptions.keys()];
    client.resetConnection(new Error("socket lost"));
    // Only connection establishment is fake; ownership/reset and replay are production.
    window.clearTimeout(client.reconnectTimeout);
    client.reconnectTimeout = null;
    entries[2].controller.abort();
    client.wsId = 8;
    await client.replayLiveSubscriptions();
    client.reconnectWaiters.settle();
    await flush();
    await tickTo(now + 60000);
    const replay = frames("REQ").filter((w) => w.socket === 8);
    assert.deepEqual(
      replay.map((w) => w.frame[1]),
      originalIds.slice(0, 2),
    );
    assert.deepEqual(
      replay.map((w) => w.frame[2]),
      entries.slice(0, 2).map((e) => e.filter),
    );
    await Promise.allSettled(entries.map((e) => e.promise));
  });
}

test("finite history and its CLOSED retry bypass a live backlog without consuming queued response budget", async () => {
  const client = session();
  start(client, 296);
  await flush();
  const filter = { kinds: [7], "#e": ["message"], limit: 1000 };
  const history = client.fetchEvents(filter);
  void history.catch(() => {});
  await flush();
  const initial = frames("REQ").find((w) => w.frame[1].startsWith("history-"));
  assert.ok(initial);
  assert.equal(initial.at, 0);
  await deliver(client, [
    "CLOSED",
    initial.frame[1],
    "rate-limited: quota exceeded; retry in 4s",
  ]);
  await tickTo(4000);
  const retry = frames("REQ").filter((w) =>
    w.frame[1].startsWith("history-"),
  )[1];
  assert.ok(retry, "history retry is not queued behind 295 live entries");
  assert.equal(retry.at, 4000);
  assert.deepEqual(retry.frame[2], filter);
  await deliver(client, ["EOSE", retry.frame[1]]);
  assert.deepEqual(await history, []);
});
