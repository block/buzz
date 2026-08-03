import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { test } from "node:test";
import {
  BoundedIpcSendQueue,
  PerKeyRequestQueue,
  RuntimeSessionProxy,
  RuntimeWorkerClient,
} from "../dist/index.js";
import { silentLogger, testConfig } from "./helpers.mjs";

class FakeChild extends EventEmitter {
  connected = true;
  killed = false;
  stdout = new EventEmitter();
  stderr = new EventEmitter();
  sent = [];
  signals = [];
  disconnectCalls = 0;
  pid = 42;

  send(message, callback) {
    this.sent.push(message);
    callback?.(null);
    return true;
  }

  disconnect() {
    this.disconnectCalls += 1;
    this.connected = false;
  }

  kill(signal = "SIGTERM") {
    this.killed = true;
    this.signals.push(signal);
    return true;
  }

  exit(code = 0, signal = null) {
    this.connected = false;
    this.emit("exit", code, signal);
  }
}

test("a prompt waits for durable parent persistence before ACKing a child lifecycle record", async () => {
  const child = new FakeChild();
  const client = new RuntimeWorkerClient(
    testConfig({ runtimeControlTimeoutMs: 1_000 }),
    silentLogger,
    () => child,
  );
  let releasePersistence;
  const persisted = new Promise((resolve) => {
    releasePersistence = resolve;
  });
  const proxy = new RuntimeSessionProxy(client, "session-lifecycle-ack", {
    sessionUpdate() {},
    async buzzSessionEvent() {
      await persisted;
    },
    usageUpdate() {},
  });
  client.register(proxy);

  let promptSettled = false;
  const prompt = proxy.prompt("continue").finally(() => {
    promptSettled = true;
  });
  await new Promise((resolve) => setImmediate(resolve));
  const promptRequest = child.sent.at(-1);
  assert.equal(promptRequest.method, "prompt");

  const deliveryId = "9ba32f72-e8ce-5195-96a2-7b472198bb7e";
  child.emit("message", {
    type: "event",
    sessionId: "session-lifecycle-ack",
    eventType: "buzz_session_event",
    deliveryId,
    payload: {
      type: "compaction_completed",
      compactionId: "a5f4cb48-cdd9-4b7d-9542-3452087f4b45",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Context compacted",
      piSessionId: "pi_test",
      reason: "threshold",
      beforeTokens: 140_000,
      afterTokens: 30_000,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      willRetry: false,
      fromExtension: false,
    },
  });
  child.emit("message", {
    type: "response",
    id: promptRequest.id,
    ok: true,
    result: "end_turn",
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(promptSettled, false);
  assert.equal(
    child.sent.some((request) => request.method === "ackLifecycle"),
    false,
  );

  releasePersistence();
  await new Promise((resolve) => setImmediate(resolve));
  const ackRequest = child.sent.at(-1);
  assert.equal(ackRequest.method, "ackLifecycle");
  assert.deepEqual(ackRequest.params, { deliveryId });
  child.emit("message", {
    type: "response",
    id: ackRequest.id,
    ok: true,
    result: { acknowledged: true },
  });
  assert.equal(await prompt, "end_turn");

  const shutdown = client.shutdown();
  await new Promise((resolve) => setImmediate(resolve));
  const shutdownRequest = child.sent.at(-1);
  child.emit("message", {
    type: "response",
    id: shutdownRequest.id,
    ok: true,
    result: { shutdown: true },
  });
  await new Promise((resolve) => setImmediate(resolve));
  child.exit(0, null);
  await shutdown;
});

test("failed runtime disposal still drains a buffered parent lifecycle handoff", async () => {
  const child = new FakeChild();
  const client = new RuntimeWorkerClient(
    testConfig({ runtimeControlTimeoutMs: 1_000 }),
    silentLogger,
    () => child,
  );
  let releasePersistence;
  const persistenceGate = new Promise((resolve) => {
    releasePersistence = resolve;
  });
  const timeline = [];
  const proxy = new RuntimeSessionProxy(client, "session-failed-drain", {
    sessionUpdate() {},
    async buzzSessionEvent() {
      await persistenceGate;
      timeline.push("parent-outbox");
    },
    usageUpdate() {},
  });
  client.register(proxy);
  const deliveryId = "9ba32f72-e8ce-5195-96a2-7b472198bb7e";
  const eventTask = proxy.handleEvent({
    type: "event",
    sessionId: "session-failed-drain",
    eventType: "buzz_session_event",
    deliveryId,
    payload: {
      type: "context_status",
      timestamp: "2026-08-02T00:00:00.000Z",
      message: "Buffered before worker failure.",
      piSessionId: "pi_test",
      usedTokens: 75_000,
      remainingTokens: 75_000,
      percent: 50,
      limitTokens: 150_000,
      effectiveLimitTokens: 150_000,
      compactionThresholdTokens: 133_616,
      autoCompaction: true,
      compacting: false,
      model: "provider/model",
    },
  });
  proxy.invalidate(
    new Error("BUZZ_PI_SESSION_INVALIDATED: worker exited"),
    Promise.resolve(),
  );
  let disposalSettled = false;
  const disposal = proxy.dispose().finally(() => {
    disposalSettled = true;
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(disposalSettled, false);

  releasePersistence();
  await assert.rejects(eventTask, /BUZZ_PI_SESSION_INVALIDATED/);
  await assert.rejects(disposal, /BUZZ_PI_SESSION_INVALIDATED/);
  assert.deepEqual(timeline, ["parent-outbox"]);
  assert.equal(
    child.sent.some((request) => request.method === "ackLifecycle"),
    false,
    "a failed child ACK cannot undo the parent-durable copy",
  );
});

test("a timed-out worker fences replacement and lease invalidation until confirmed exit", async () => {
  const children = [];
  const invalidations = [];
  const client = new RuntimeWorkerClient(
    testConfig({ runtimeControlTimeoutMs: 20, runtimeInterruptTimeoutMs: 5 }),
    silentLogger,
    () => {
      const child = new FakeChild();
      children.push(child);
      return child;
    },
  );
  client.register({
    acpSessionId: "old",
    invalidate() {},
    handleEvent() {},
  });
  client.setInvalidationHandler(async (sessionIds) => {
    invalidations.push([...sessionIds]);
  });

  const oldRequest = client.request("create", "old", { cwd: "/tmp" });
  const oldOutcome = oldRequest.then(
    () => ({ settled: true, error: undefined }),
    (error) => ({ settled: true, error }),
  );
  let oldSettled = false;
  void oldOutcome.then(() => {
    oldSettled = true;
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  const oldChild = children[0];
  assert.deepEqual(oldChild.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(oldSettled, false);
  assert.deepEqual(invalidations, []);

  const replacementRequest = client.request("create", "replacement", {
    cwd: "/tmp",
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    children.length,
    1,
    "replacement spawned before predecessor exit",
  );

  oldChild.exit(1, "SIGKILL");
  const outcome = await oldOutcome;
  assert.match(outcome.error.message, /timed out/);
  assert.deepEqual(invalidations, [["old"]]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(children.length, 2);
  const replacement = children[1];
  const request = replacement.sent.at(-1);
  replacement.emit("message", {
    type: "response",
    id: request.id,
    ok: true,
    result: { generation: "replacement" },
  });
  assert.deepEqual((await replacementRequest).result, {
    generation: "replacement",
  });

  const shutdown = client.shutdown();
  await new Promise((resolve) => setImmediate(resolve));
  const shutdownRequest = replacement.sent.at(-1);
  replacement.emit("message", {
    type: "response",
    id: shutdownRequest.id,
    ok: true,
    result: { shutdown: true },
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(replacement.disconnectCalls, 1);
  replacement.exit(0, null);
  await shutdown;
});

test("retirement escalates an ignored SIGTERM to SIGKILL without spawning", async () => {
  const children = [];
  const client = new RuntimeWorkerClient(
    testConfig({ runtimeControlTimeoutMs: 10, runtimeInterruptTimeoutMs: 5 }),
    silentLogger,
    () => {
      const child = new FakeChild();
      children.push(child);
      return child;
    },
  );

  const failed = client.request("create", "stuck", { cwd: "/tmp" });
  await new Promise((resolve) => setTimeout(resolve, 15));
  const waitingReplacement = client.request("create", "next", {
    cwd: "/tmp",
  });
  await new Promise((resolve) => setTimeout(resolve, 15));
  assert.deepEqual(children[0].signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(children.length, 1);

  children[0].exit(1, "SIGKILL");
  await assert.rejects(() => failed, /timed out/);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(children.length, 2);
  const nextRequest = children[1].sent.at(-1);
  children[1].emit("message", {
    type: "response",
    id: nextRequest.id,
    ok: true,
    result: { generation: "next" },
  });
  assert.equal((await waitingReplacement).result.generation, "next");

  const shutdown = client.shutdown();
  await new Promise((resolve) => setImmediate(resolve));
  const shutdownRequest = children[1].sent.at(-1);
  children[1].emit("message", {
    type: "response",
    id: shutdownRequest.id,
    ok: true,
    result: { shutdown: true },
  });
  await new Promise((resolve) => setImmediate(resolve));
  children[1].exit(0, null);
  await shutdown;
});

test("worker crash rejects all outstanding requests as one failed generation", async () => {
  const child = new FakeChild();
  const client = new RuntimeWorkerClient(
    testConfig({ runtimeControlTimeoutMs: 1_000 }),
    silentLogger,
    () => child,
  );
  const first = client.request("create", "one", { cwd: "/tmp" });
  const second = client.request("create", "two", { cwd: "/tmp" });
  child.exit(9, null);
  await assert.rejects(() => first, /exited/);
  await assert.rejects(() => second, /exited/);
});

test("runtime host queue serializes one session while allowing distinct sessions in parallel", async () => {
  const queue = new PerKeyRequestQueue();
  const order = [];
  let releaseFirst;
  const gate = new Promise((resolve) => {
    releaseFirst = resolve;
  });
  const first = queue.run("same", async () => {
    order.push("same-1-start");
    await gate;
    order.push("same-1-end");
  });
  const second = queue.run("same", async () => {
    order.push("same-2");
  });
  const other = queue.run("other", async () => {
    order.push("other");
  });
  await other;
  assert.deepEqual(order, ["same-1-start", "other"]);
  releaseFirst();
  await Promise.all([first, second]);
  assert.deepEqual(order, ["same-1-start", "other", "same-1-end", "same-2"]);
});

test("bounded IPC sender honors callbacks after false returns and never reorders frames", async () => {
  const sent = [];
  const callbacks = [];
  const failures = [];
  const sender = new BoundedIpcSendQueue(
    (message, callback) => {
      sent.push(message);
      callbacks.push(callback);
      return false;
    },
    4,
    1_024,
    (error) => failures.push(error),
  );

  assert.equal(sender.enqueue({ sequence: 1 }), true);
  assert.equal(sender.enqueue({ sequence: 2 }), true);
  assert.deepEqual(sent, [{ sequence: 1 }]);
  callbacks.shift()(null);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(sent, [{ sequence: 1 }, { sequence: 2 }]);
  callbacks.shift()(null);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(failures, []);
});

test("bounded IPC sender poisons the generation on saturation", () => {
  const failures = [];
  const sender = new BoundedIpcSendQueue(
    () => false,
    2,
    1_024,
    (error) => failures.push(error),
  );
  assert.equal(sender.enqueue({ sequence: 1 }), true);
  assert.equal(sender.enqueue({ sequence: 2 }), true);
  assert.equal(sender.enqueue({ sequence: 3 }), false);
  assert.equal(failures.length, 1);
  assert.match(failures[0].message, /queue saturated/);
  assert.equal(sender.enqueue({ sequence: 4 }), false);
  assert.equal(failures.length, 1);
});

test("bounded IPC sender drains a burst in exact insertion order", async () => {
  const sent = [];
  const sender = new BoundedIpcSendQueue(
    (message, callback) => {
      sent.push(message.sequence);
      queueMicrotask(() => callback(null));
      return sent.length % 2 === 0;
    },
    256,
    64 * 1_024,
    (error) => assert.fail(error.message),
  );
  for (let sequence = 0; sequence < 128; sequence += 1) {
    assert.equal(sender.enqueue({ sequence }), true);
  }
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    sent,
    Array.from({ length: 128 }, (_, index) => index),
  );
});

test("bounded IPC sender permits shared aliases that Node IPC serializes", async () => {
  const shared = { id: "shared-model-capability" };
  const sent = [];
  const failures = [];
  const sender = new BoundedIpcSendQueue(
    (message, callback) => {
      // Model Node's JSON IPC serializer: sibling aliases are duplicated and
      // accepted, even though a permanent WeakSet would reject them.
      sent.push(JSON.parse(JSON.stringify(message)));
      queueMicrotask(() => callback(null));
      return true;
    },
    4,
    1_024,
    (error) => failures.push(error),
  );

  assert.equal(
    sender.enqueue({
      models: [{ capability: shared }, { capability: shared }],
    }),
    true,
  );
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(sent[0].models[0].capability, shared);
  assert.deepEqual(sent[0].models[1].capability, shared);
  assert.deepEqual(failures, []);
});

test("bounded IPC sender still rejects a true recursive cycle", () => {
  const failures = [];
  const cyclic = { id: "cycle" };
  cyclic.self = cyclic;
  const sender = new BoundedIpcSendQueue(
    () => true,
    4,
    1_024,
    (error) => failures.push(error),
  );

  assert.equal(sender.enqueue(cyclic), false);
  assert.equal(failures.length, 1);
  assert.match(failures[0].message, /contains a cycle/);
});

test("bounded IPC sender rejects advanced serializer values instead of undercounting them", () => {
  class CustomFrame {
    value = "custom";
  }
  const advanced = [
    ["ArrayBuffer", new ArrayBuffer(1_024)],
    ["typed-array/view", new Uint8Array(1_024)],
    ["typed-array/view", new DataView(new ArrayBuffer(32))],
    ["Map", new Map([["key", "value"]])],
    ["Set", new Set(["value"])],
    ["Date", new Date()],
    ["non-plain object", new CustomFrame()],
  ];
  if (typeof SharedArrayBuffer !== "undefined") {
    advanced.push(["SharedArrayBuffer", new SharedArrayBuffer(1_024)]);
  }

  for (const [kind, value] of advanced) {
    const failures = [];
    const sender = new BoundedIpcSendQueue(
      () => true,
      4,
      128,
      (error) => failures.push(error),
    );
    assert.equal(sender.enqueue({ value }), false, kind);
    assert.equal(failures.length, 1, kind);
    assert.match(failures[0].message, new RegExp(kind));
  }
});

test("bounded IPC sender measures JSON escaping and snapshots queued frames", async () => {
  const escapingFailures = [];
  const escaping = new BoundedIpcSendQueue(
    () => true,
    4,
    32,
    (error) => escapingFailures.push(error),
  );
  assert.equal(escaping.enqueue({ text: "\u0000".repeat(10) }), false);
  assert.match(escapingFailures[0].message, /exceeds 32 bytes/);

  const sent = [];
  const callbacks = [];
  const sender = new BoundedIpcSendQueue(
    (message, callback) => {
      sent.push(message);
      callbacks.push(callback);
      return false;
    },
    4,
    1_024,
    (error) => assert.fail(error.message),
  );
  const first = { sequence: 1 };
  const queued = { sequence: 2, nested: { value: "before" } };
  assert.equal(sender.enqueue(first), true);
  assert.equal(sender.enqueue(queued), true);
  queued.nested.value = "after";
  callbacks.shift()(null);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(sent[1], {
    sequence: 2,
    nested: { value: "before" },
  });
  callbacks.shift()(null);
});

test("bounded IPC sender caps inherited enumerable array decoration scans", () => {
  const prototype = {};
  for (let index = 0; index < 9_000; index += 1) {
    prototype[`inherited_${index}`] = index;
  }
  const decorated = [];
  Object.setPrototypeOf(decorated, prototype);
  const failures = [];
  const sender = new BoundedIpcSendQueue(
    () => true,
    4,
    1_024 * 1_024,
    (error) => failures.push(error),
  );
  assert.equal(sender.enqueue({ decorated }), false);
  assert.equal(failures.length, 1);
  assert.match(failures[0].message, /8192 enumerated keys/);
});
