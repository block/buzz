// Exercise actual session cancellation at the IPC boundary, including in-flight sends.
// The IPC transport and clock are fake; no native app or external relay is used.
import assert from "node:assert/strict";
import { getEventListeners } from "node:events";
import { after, beforeEach, afterEach, test } from "node:test";

const originalNow = Date.now;
const originalWindow = globalThis.window;
let now = 0;
let nextTimerId = 1;
const timers = new Map();
const writes = [];
const clients = [];
let sendHook = async () => {};
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
        return sendHook(args, JSON.parse(args.message.data));
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
  sendHook = async () => {};
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
const filter = { kinds: [9], "#h": ["channel"], since: 123, limit: 1000 };
function setup() {
  const client = new RelayClient();
  client.wsId = 7;
  clients.push(client);
  const controller = new AbortController();
  let outcome = "pending";
  const readiness = [];
  const events = [];
  const start = () => {
    const pending = client.subscribeLive(
      filter,
      (e) => events.push(e),
      (r) => readiness.push(r),
      250,
      controller.signal,
    );
    pending.then(
      () => {
        outcome = "ready";
      },
      (e) => {
        outcome = e.name;
      },
    );
    return pending;
  };
  return {
    client,
    controller,
    start,
    readiness,
    events,
    outcome: () => outcome,
  };
}
for (const stop of ["abort", "disconnect"]) {
  test(`${stop} settles setup while connection is pending, without late registration`, async () => {
    const h = setup();
    const connection = Promise.withResolvers();
    h.client.connectPromise = connection.promise;
    h.start();
    if (stop === "abort") h.controller.abort();
    else h.client.disconnect();
    await flush();
    try {
      assert.notEqual(h.outcome(), "pending");
      h.client.wsId = 8;
      connection.resolve(h.client.connectionGeneration);
      await flush();
      assert.equal(frames("REQ").length, 0);
      assert.equal(h.client.subscriptions.size, 0);
    } finally {
      connection.resolve(h.client.connectionGeneration);
      h.client.connectPromise = null;
      await flush();
    }
  });
}
test("abort during an in-flight REQ suppresses delivery and closes again after send settles", async () => {
  const h = setup();
  const send = Promise.withResolvers();
  sendHook = async (_args, f) => {
    if (f[0] === "REQ") await send.promise;
  };
  h.start();
  await flush();
  const id = frames("REQ")[0].frame[1];
  h.controller.abort();
  await flush();
  try {
    assert.equal(h.client.subscriptions.size, 0);
    assert.equal(h.outcome(), "AbortError");
    const before = frames("CLOSE").length;
    assert.ok(before > 0);
    await deliver(h.client, [
      "EVENT",
      id,
      { id: "event", kind: 9, created_at: 1 },
    ]);
    await deliver(h.client, ["EOSE", id]);
    assert.equal(h.events.length, 0);
    send.resolve();
    await flush();
    assert.ok(
      frames("CLOSE").length > before,
      "post-flight close preserves wire order",
    );
    assert.equal(h.client.wsId, 7);
    assert.deepEqual(h.readiness, []);
  } finally {
    send.resolve();
    await flush();
  }
});
for (const stage of ["first send", "reconnect wait", "retry send"]) {
  test(`cancellation at ${stage} cannot reset or resurrect the replacement socket`, async () => {
    const h = setup();
    const first = Promise.withResolvers();
    const retry = Promise.withResolvers();
    let attempts = 0;
    sendHook = async (_args, f) => {
      if (f[0] !== "REQ") return;
      await (++attempts === 1 ? first.promise : retry.promise);
    };
    h.start();
    await flush();
    try {
      if (stage === "first send") {
        h.controller.abort();
        first.reject(new Error("late IPC error"));
        await flush();
        assert.equal(h.client.wsId, 7);
        assert.equal(h.client.reconnectTimeout, null);
      } else {
        first.reject(new Error("socket lost"));
        await flush();
        assert.notEqual(h.client.reconnectTimeout, null);
        if (stage === "reconnect wait") h.controller.abort();
        // Resolve the existing reconnect waiter: only transport establishment is fake.
        window.clearTimeout(h.client.reconnectTimeout);
        h.client.reconnectTimeout = null;
        h.client.wsId = 8;
        h.client.reconnectWaiters.settle();
        await flush();
        if (stage === "retry send") {
          assert.equal(attempts, 2);
          h.controller.abort();
          retry.reject(new Error("late retry IPC error"));
          await flush();
        } else assert.equal(attempts, 1);
        assert.equal(h.client.wsId, 8);
      }
      assert.equal(h.client.subscriptions.size, 0);
      assert.equal(h.outcome(), "AbortError");
    } finally {
      first.resolve();
      retry.resolve();
      await flush();
    }
  });
}
for (const finish of ["EOSE", "timeout", "terminal CLOSED", "disconnect"]) {
  test(`${finish} settles readiness once and cleans up cancellation`, async () => {
    const h = setup();
    h.start();
    await flush();
    const id = frames("REQ")[0].frame[1];
    if (finish === "EOSE") {
      await deliver(h.client, ["EOSE", id]);
      await deliver(h.client, ["EOSE", id]);
    }
    if (finish === "timeout") await tickTo(250);
    if (finish === "terminal CLOSED")
      await deliver(h.client, ["CLOSED", id, "restricted: denied"]);
    if (finish === "disconnect") h.client.disconnect();
    await flush();
    assert.notEqual(h.outcome(), "pending");
    assert.ok(h.readiness.length <= 1);
    assert.equal(timers.size, 0);
    h.controller.abort();
    await flush();
    assert.equal(h.client.subscriptions.size, 0);
  });
}

for (const stop of ["abort", "disconnect"]) {
  test(`${stop} cancels an actual CLOSED retry while its send is in flight`, async () => {
    const h = setup();
    const started = h.start();
    await flush();
    const id = frames("REQ")[0].frame[1];
    await deliver(h.client, ["EOSE", id]);
    await started;
    await deliver(h.client, ["CLOSED", id, "error: temporary failure"]);
    const retry = Promise.withResolvers();
    sendHook = async (_args, f) => {
      if (f[0] === "REQ") await retry.promise;
    };
    await tickTo(1000);
    assert.equal(frames("REQ").length, 2);
    if (stop === "abort") h.controller.abort();
    else {
      h.client.disconnect();
      h.client.wsId = 8;
    }
    retry.reject(new Error("late CLOSED retry failure"));
    await flush();
    await tickTo(60000);
    assert.equal(frames("REQ").length, 2);
    assert.equal(h.client.wsId, stop === "abort" ? 7 : 8);
    assert.equal(h.client.subscriptions.size, 0);
    assert.equal(timers.size, 0);
  });
}
for (const stop of ["abort", "dispose", "terminal CLOSED", "disconnect"]) {
  test(`${stop} releases caller and session abort listeners after EOSE`, async () => {
    const h = setup();
    const sessionSignal = h.client.liveSessionAbort.signal;
    const started = h.start();
    await flush();
    const id = frames("REQ")[0].frame[1];
    await deliver(h.client, ["EOSE", id]);
    const dispose = await started;
    assert.equal(
      getEventListeners(h.controller.signal, "abort").length,
      1,
      "live cancellation remains armed after readiness",
    );
    if (stop === "abort") h.controller.abort();
    if (stop === "dispose") await dispose();
    if (stop === "terminal CLOSED")
      await deliver(h.client, ["CLOSED", id, "restricted: denied"]);
    if (stop === "disconnect") h.client.disconnect();
    await flush();
    assert.equal(getEventListeners(h.controller.signal, "abort").length, 0);
    assert.equal(getEventListeners(sessionSignal, "abort").length, 0);
    assert.equal(timers.size, 0);
  });
}
test("workspace switch during in-flight setup cannot close or reset the new socket", async () => {
  const h = setup();
  const send = Promise.withResolvers();
  sendHook = async (_args, f) => {
    if (f[0] === "REQ") await send.promise;
  };
  h.start();
  await flush();
  h.client.disconnect();
  h.client.wsId = 8;
  send.reject(new Error("old workspace send failed"));
  await flush();
  assert.equal(h.outcome(), "AbortError");
  assert.equal(h.client.wsId, 8);
  assert.ok(writes.every((w) => w.socket === 7));
  assert.equal(h.client.subscriptions.size, 0);
  assert.equal(timers.size, 0);
});

test("late old-socket failure preserves a still-owned live entry restored by replay", async () => {
  const h = setup();
  const first = Promise.withResolvers();
  let requests = 0;
  sendHook = async (_args, f) => {
    if (f[0] === "REQ" && ++requests === 1) await first.promise;
  };
  h.start();
  await flush();
  const id = frames("REQ")[0].frame[1];
  const entry = h.client.subscriptions.get(id);
  h.client.resetConnection(new Error("socket closed during setup"));
  window.clearTimeout(h.client.reconnectTimeout);
  h.client.reconnectTimeout = null;
  h.client.wsId = 8;
  await h.client.replayLiveSubscriptions();
  assert.equal(frames("REQ").length, 2);
  assert.equal(frames("REQ")[1].frame[1], id);
  first.reject(new Error("late old-socket IPC failure"));
  await flush();
  assert.equal(h.outcome(), "ready");
  assert.equal(h.client.subscriptions.get(id), entry);
  assert.equal(frames("CLOSE").length, 0);
  assert.equal(h.client.wsId, 8);
  assert.equal(frames("REQ").length, 2, "replay, not fresh setup");
});

test("late retry-send failure preserves an entry handed to another reconnect", async () => {
  const h = setup();
  const retry = Promise.withResolvers();
  let requests = 0;
  sendHook = async (_args, f) => {
    if (f[0] !== "REQ") return;
    if (++requests === 1) throw new Error("first socket failed");
    if (requests === 2) await retry.promise;
  };
  h.start();
  await flush();
  window.clearTimeout(h.client.reconnectTimeout);
  h.client.reconnectTimeout = null;
  h.client.wsId = 8;
  h.client.reconnectWaiters.settle();
  await flush();
  assert.equal(requests, 2);
  const id = frames("REQ")[0].frame[1];
  const entry = h.client.subscriptions.get(id);
  h.client.resetConnection(new Error("retry socket closed"));
  window.clearTimeout(h.client.reconnectTimeout);
  h.client.reconnectTimeout = null;
  h.client.wsId = 9;
  await h.client.replayLiveSubscriptions();
  retry.reject(new Error("late retry-socket failure"));
  await flush();
  assert.equal(h.outcome(), "ready");
  assert.equal(h.client.subscriptions.get(id), entry);
  assert.equal(frames("CLOSE").length, 0);
  assert.equal(h.client.wsId, 9);
});

test("a failed reconnect attempt still rejects setup rather than claiming a successful handoff", async () => {
  const h = setup();
  sendHook = async (_args, f) => {
    if (f[0] === "REQ") throw new Error("socket failed");
  };
  h.start();
  await flush();
  window.clearTimeout(h.client.reconnectTimeout);
  h.client.reconnectTimeout = null;
  h.client.reconnectWaiters.settle(new Error("authentication failed"));
  await flush();
  assert.equal(h.outcome(), "Error");
  assert.equal(h.client.subscriptions.size, 0);
  assert.equal(frames("REQ").length, 1);
  assert.equal(timers.size, 0);
});
