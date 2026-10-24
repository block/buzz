// Exercise real subscribeLive -> inbound CLOSED -> retry timer -> Tauri send.
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
const { resetRateLimitGate, isRateLimited } = await import(
  "./relayRateLimitGate.ts"
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
async function liveChannels(count = 1) {
  const client = new RelayClient();
  client.wsId = 7; // Already authenticated; subscribe and dispatch remain real.
  clients.push(client);
  const filters = Array.from({ length: count }, (_, i) => ({
    kinds: [9, 5, 7],
    "#h": [`channel-${i}`],
    since: 123,
    limit: 1000,
  }));
  const ready = filters.map((filter) => client.subscribeLive(filter, () => {}));
  for (const pending of ready) void pending.catch(() => {});
  await flush();
  await tickTo(now + (count - 1) * 250);
  const initial = frames("REQ").slice(-count);
  assert.equal(initial.length, count);
  for (const { frame } of initial) await deliver(client, ["EOSE", frame[1]]);
  const disposers = await Promise.all(ready);
  return {
    client,
    filters,
    ids: initial.map(({ frame }) => frame[1]),
    disposers,
  };
}
async function refuse(client, id) {
  await deliver(client, [
    "CLOSED",
    id,
    "rate-limited: quota exceeded; retry in 4s",
  ]);
}
async function extend(client, seconds) {
  await deliver(client, [
    "NOTICE",
    `rate-limited: quota exceeded; retry in ${seconds}s`,
  ]);
}

test("154 real live retries and a publish respect a repeatedly extended cooldown", async () => {
  const { client, ids, filters } = await liveChannels(154);
  const base = now;
  for (const id of ids) await refuse(client, id);
  const event = { id: "a".repeat(64), kind: 9 };
  const published = client.publishEvent(event, "timeout", "send failed");
  // Cleanup disconnects on an assertion failure; don't leak its rejection.
  void published.catch(() => {});
  await tickTo(base + 1000);
  await extend(client, 10); // Original retries wake at 4s; gate now ends at 11s.
  await tickTo(base + 4000);
  assert.equal(frames("REQ").length, 154, "no retry through an extended gate");
  assert.equal(frames("EVENT").length, 0);
  assert.equal(
    client.pendingEvents.size,
    0,
    "publish budget starts after gate",
  );
  for (const id of ids) {
    assert.equal(client.subscriptions.get(id).closedRetryAttempt, 1);
  }
  await tickTo(base + 5000);
  await extend(client, 10); // Extend again while the replacement timers sleep.
  await tickTo(base + 11000);
  assert.equal(frames("REQ").length, 154);
  assert.equal(frames("EVENT").length, 0);
  await tickTo(base + 14999);
  assert.equal(frames("REQ").length, 154);
  await tickTo(base + 15000);
  assert.equal(isRateLimited(), false);
  assert.equal(
    frames("REQ").length,
    155,
    "only one live subscription retries alongside Send",
  );
  assert.equal(frames("EVENT").length, 1);
  await deliver(client, ["OK", event.id, true, ""]);
  assert.equal(await published, event);
  await tickTo(base + 15000 + 153 * 250);
  assert.equal(
    frames("REQ").length,
    308,
    "each live subscription retries once",
  );
  assert.deepEqual(
    frames("REQ")
      .slice(154)
      .map(({ frame }) => frame.slice(1)),
    ids.map((id, i) => [id, filters[i]]),
    "retry preserves coverage/filter and ID",
  );
  assert.ok(
    frames("REQ")
      .slice(154)
      .every(({ at }, i) => at === base + 15000 + i * 250),
  );
  for (const id of ids) await deliver(client, ["EOSE", id]);
  await tickTo(base + 60000);
  assert.equal(frames("REQ").length, 308, "EOSE leaves no extra retry");
});

for (const stop of ["dispose", "EOSE", "terminal CLOSED", "workspace switch"]) {
  test(`${stop} cancels a live retry re-armed behind the cooldown`, async () => {
    const {
      client,
      ids: [id],
      disposers: [dispose],
    } = await liveChannels();
    await refuse(client, id);
    await tickTo(1000);
    await extend(client, 10);
    await tickTo(4000);
    assert.equal(frames("REQ").length, 1, "retry must still be pending");
    if (stop === "dispose") await dispose();
    if (stop === "EOSE") await deliver(client, ["EOSE", id]);
    if (stop === "terminal CLOSED")
      await deliver(client, ["CLOSED", id, "restricted: denied"]);
    if (stop === "workspace switch") {
      client.disconnect();
      resetRateLimitGate();
      client.wsId = 8;
    }
    await tickTo(60000);
    assert.equal(frames("REQ").length, 1, "retired retry must not send");
    assert.ok(!writes.some(({ socket }) => socket === 8));
  });
}

test("a non-quota live retry also respects a later shared cooldown", async () => {
  const {
    client,
    ids: [id],
  } = await liveChannels();
  await deliver(client, ["CLOSED", id, "error: temporary failure"]);
  await tickTo(500);
  await extend(client, 4);
  await tickTo(1000);
  assert.equal(frames("REQ").length, 1);
  await tickTo(4500);
  assert.equal(frames("REQ").length, 2);
  await deliver(client, ["EOSE", id]);
});

async function historyRequest() {
  const client = new RelayClient();
  client.wsId = 7;
  clients.push(client);
  const filter = { kinds: [7], "#e": ["target"], since: 123, limit: 10000 };
  const history = client.fetchEvents(filter);
  void history.catch(() => {});
  await flush();
  const id = frames("REQ").at(-1).frame[1];
  return { client, history, filter, id };
}
function historyId(client) {
  return [...client.subscriptions].find(([, sub]) => sub.mode === "history")[0];
}

test("history retry and Send respect extended deadlines without consuming response budget", async () => {
  const { client, history, filter, id } = await historyRequest();
  const partial = { id: "partial", kind: 7, created_at: 10, tags: [] };
  await deliver(client, ["EVENT", id, partial]);
  await refuse(client, id);
  const retryId = historyId(client);
  const subscription = client.subscriptions.get(retryId);
  const event = { id: "b".repeat(64), kind: 9 };
  const published = client.publishEvent(event, "timeout", "send failed");
  void published.catch(() => {});
  await tickTo(1000);
  await extend(client, 30); // Wait exceeds the normal 25s response timeout.
  await tickTo(4000);
  assert.equal(
    frames("REQ").length,
    1,
    "history cannot retry through extended gate",
  );
  await tickTo(5000);
  await extend(client, 30); // deadline now 35s, not 31s.
  await tickTo(31000);
  assert.equal(frames("REQ").length, 1);
  assert.equal(client.subscriptions.get(retryId), subscription);
  assert.equal(subscription.closedRetryAttempt, 1, "waiting is not an attempt");
  assert.equal(frames("EVENT").length, 0);
  assert.equal(client.pendingEvents.size, 0);
  await tickTo(34999);
  assert.equal(frames("REQ").length, 1);
  await tickTo(35000);
  assert.deepEqual(frames("REQ").at(-1).frame, ["REQ", retryId, filter]);
  assert.equal(frames("REQ").at(-1).at, 35000);
  assert.equal(frames("EVENT").length, 1);
  await tickTo(40000); // Plenty of response budget left after the long wait.
  await deliver(client, ["EOSE", retryId]);
  assert.deepEqual(await history, [partial]);
  await deliver(client, ["OK", event.id, true, ""]);
  assert.equal(await published, event);
});

for (const stop of ["workspace switch", "terminal CLOSED", "EOSE"]) {
  test(`${stop} cancels history retry re-armed behind the gate`, async () => {
    const { client, history, id } = await historyRequest();
    await refuse(client, id);
    await tickTo(1000);
    await extend(client, 10);
    await tickTo(4000);
    assert.equal(frames("REQ").length, 1);
    const retryId = historyId(client);
    if (stop === "workspace switch") {
      client.disconnect();
      resetRateLimitGate();
      client.wsId = 8;
      await assert.rejects(history, /community switch/);
    } else if (stop === "terminal CLOSED") {
      await deliver(client, ["CLOSED", retryId, "restricted: denied"]);
      await assert.rejects(history, /restricted/);
    } else {
      await deliver(client, ["EOSE", retryId]);
      assert.deepEqual(await history, []);
    }
    await tickTo(60000);
    assert.equal(frames("REQ").length, 1);
    assert.ok(!writes.some(({ socket }) => socket === 8));
  });
}

test("zero-second history refusal does not hold Send for the missing-hint default", async () => {
  const { client, history, id } = await historyRequest();
  await deliver(client, [
    "CLOSED",
    id,
    "rate-limited: quota exceeded; retry in 0s",
  ]);
  const event = { id: "c".repeat(64), kind: 9 };
  const published = client.publishEvent(event, "timeout", "send failed");
  void published.catch(() => {});
  await tickTo(0);
  assert.equal(isRateLimited(), false);
  assert.equal(frames("REQ").length, 2);
  assert.equal(frames("EVENT").length, 1);
  await deliver(client, ["EOSE", historyId(client)]);
  assert.deepEqual(await history, []);
  await deliver(client, ["OK", event.id, true, ""]);
  assert.equal(await published, event);
});

test("repeated zero-second history refusals exhaust the existing three-retry budget", async () => {
  const { client, history, id } = await historyRequest();
  onSend = ({ frame }) =>
    frame[0] === "REQ"
      ? deliver(client, [
          "CLOSED",
          frame[1],
          "rate-limited: quota exceeded; retry in 0s",
        ])
      : undefined;
  await deliver(client, [
    "CLOSED",
    id,
    "rate-limited: quota exceeded; retry in 0s",
  ]);
  await tickTo(0);
  assert.equal(
    frames("REQ").length,
    4,
    "initial request plus three retries, not an infinite loop",
  );
  await assert.rejects(history, /quota exceeded/);
  assert.equal(client.subscriptions.size, 0);
  assert.equal(timers.size, 0);
});

test("immediate retry EOSE settles history without leaving an op-timeout", async () => {
  const { client, history, id } = await historyRequest();
  onSend = ({ frame }) =>
    frame[0] === "REQ" ? deliver(client, ["EOSE", frame[1]]) : undefined;
  await refuse(client, id);
  await tickTo(4000);
  assert.deepEqual(await history, []);
  assert.equal(
    timers.size,
    0,
    "response can arrive before the send promise settles",
  );
});

test("history response timeout starts at admitted retry dispatch and sends CLOSE", async () => {
  const { client, history, id } = await historyRequest();
  await refuse(client, id);
  await tickTo(1000);
  await extend(client, 30);
  await tickTo(31000);
  const retryId = historyId(client);
  assert.equal(frames("REQ").at(-1).at, 31000);
  await tickTo(55999);
  assert.ok(client.subscriptions.has(retryId));
  await tickTo(56000);
  await assert.rejects(history, /closed the history/);
  assert.ok(frames("CLOSE").some(({ frame }) => frame[1] === retryId));
  assert.equal(client.subscriptions.size, 0);
});

test("late failed history send cannot cancel the next rotated retry", async () => {
  const { client, history, id } = await historyRequest();
  let rejectOldSend;
  onSend = ({ frame }) =>
    frame[0] === "REQ"
      ? new Promise((_, reject) => {
          rejectOldSend = reject;
        })
      : undefined;
  await refuse(client, id);
  await tickTo(4000);
  const oldId = historyId(client);
  await refuse(client, oldId);
  const nextId = historyId(client);
  const nextSub = client.subscriptions.get(nextId);
  const nextTimer = nextSub.timeout;
  onSend = undefined;
  rejectOldSend(new Error("late old IPC failure"));
  await flush();
  assert.equal(client.subscriptions.get(nextId), nextSub);
  assert.ok(
    timers.has(nextTimer),
    "late failure must not clear the next retry timer",
  );
  assert.equal(
    client.connectionGeneration,
    0,
    "retired send must not reset socket",
  );
  await tickTo(8000);
  assert.equal(frames("REQ").length, 3);
  assert.equal(frames("REQ").at(-1).frame[1], nextId);
  await deliver(client, ["EOSE", nextId]);
  assert.deepEqual(await history, []);
});

test("zero-second live refusals retain existing exponential retry backoff", async () => {
  const {
    client,
    ids: [id],
  } = await liveChannels();
  onSend = ({ frame }) =>
    frame[0] === "REQ"
      ? deliver(client, [
          "CLOSED",
          frame[1],
          "rate-limited: quota exceeded; retry in 0s",
        ])
      : undefined;
  await deliver(client, [
    "CLOSED",
    id,
    "rate-limited: quota exceeded; retry in 0s",
  ]);
  await tickTo(999);
  assert.equal(frames("REQ").length, 1);
  await tickTo(1000);
  assert.equal(frames("REQ").length, 2);
  await tickTo(3000);
  assert.equal(frames("REQ").length, 3);
  await tickTo(7000);
  assert.equal(frames("REQ").length, 4);
  assert.equal(isRateLimited(), false);
  await deliver(client, ["EOSE", id]);
});

test("zero-second negative OK remains a publish error, not successful acceptance", async () => {
  const { client } = await liveChannels();
  const event = { id: "d".repeat(64), kind: 9 };
  const published = client.publishEvent(event, "timeout", "send failed");
  void published.catch(() => {});
  await flush();
  await deliver(client, [
    "OK",
    event.id,
    false,
    "rate-limited: quota exceeded; retry in 0s",
  ]);
  await assert.rejects(published, /quota exceeded/);
  assert.equal(
    frames("EVENT").length,
    1,
    "no automatic uncertain publish replay",
  );
  assert.equal(isRateLimited(), false);
});

test("zero hint on a history retry preserves another operation's active deadline", async () => {
  const { client, history, id } = await historyRequest();
  await extend(client, 4);
  await tickTo(3000);
  await deliver(client, [
    "CLOSED",
    id,
    "rate-limited: quota exceeded; retry in 0s",
  ]);
  await tickTo(3999);
  assert.equal(frames("REQ").length, 1);
  await tickTo(4000);
  assert.equal(frames("REQ").length, 2);
  await deliver(client, ["EOSE", historyId(client)]);
  assert.deepEqual(await history, []);
});
