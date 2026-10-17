import assert from "node:assert/strict";
import test from "node:test";
import { HuddleMediaClock } from "./huddleMediaClock.ts";
import { startScreenPublisher } from "./screenPublisher.ts";

function workerHarness(t) {
  const original = globalThis.Worker;
  const workers = [];
  class FakeWorker {
    sent = [];
    terminated = false;
    constructor() {
      workers.push(this);
    }
    postMessage(message) {
      this.sent.push(message);
    }
    reply(id) {
      this.onmessage?.({ data: id });
    }
    terminate() {
      this.terminated = true;
    }
  }
  globalThis.Worker = FakeWorker;
  t.after(() => {
    globalThis.Worker = original;
  });
  return workers;
}

test("media clocks use worker wake-ups and ignore duplicate or cancelled responses", (t) => {
  const workers = workerHarness(t);
  const clock = new HuddleMediaClock(assert.fail);
  t.after(() => clock.dispose());
  let called = 0;
  const first = clock.schedule(() => called++, 0);
  const cancelled = clock.schedule(() => called++, 33);
  assert.deepEqual(
    workers[0].sent.map(({ id, delay }) => ({ id, delay })),
    [
      { id: first, delay: 0 },
      { id: cancelled, delay: 33 },
    ],
  );
  clock.unschedule(cancelled);
  workers[0].reply(cancelled);
  workers[0].reply(first);
  workers[0].reply(first);
  assert.equal(called, 1);
});

test("media clocks bound timers and reject invalid delays", (t) => {
  workerHarness(t);
  const clock = new HuddleMediaClock(assert.fail);
  t.after(() => clock.dispose());
  for (const delay of [-1, Number.NaN, Number.POSITIVE_INFINITY, 60_001])
    assert.throws(() => clock.schedule(() => {}, delay), /Invalid/);
  for (let index = 0; index < 32; index++) clock.schedule(() => {}, 10);
  assert.throws(() => clock.schedule(() => {}, 10), /excessive/);
});

test("zero-delay publication still yields instead of creating a busy IPC loop", (t) => {
  const workers = workerHarness(t);
  const previous = Object.getOwnPropertyDescriptor(globalThis, "performance");
  Object.defineProperty(globalThis, "performance", {
    configurable: true,
    value: { now: () => 20, timeOrigin: 1000 },
  });
  t.after(() => Object.defineProperty(globalThis, "performance", previous));
  const clock = new HuddleMediaClock(assert.fail);
  t.after(() => clock.dispose());
  clock.schedule(() => {}, 0);
  assert.equal(workers[0].sent[0].deadline, 1024);
});

test("disposing a clock retires every pending callback and its worker", (t) => {
  const workers = workerHarness(t);
  const clock = new HuddleMediaClock(assert.fail);
  const id = clock.schedule(assert.fail, 33);
  const queuedReply = workers[0].onmessage;
  clock.dispose();
  queuedReply({ data: id });
  clock.dispose();
  assert.equal(workers[0].terminated, true);
  assert.throws(() => clock.schedule(assert.fail, 0), /closed/);
});

test("a worker failure stops the clock and reaches the producer exactly once", (t) => {
  const workers = workerHarness(t);
  const failures = [];
  const clock = new HuddleMediaClock((error) => failures.push(error.message));
  t.after(() => clock.dispose());
  const fail = workers[0].onerror;
  fail({ message: "Worker failed", preventDefault() {} });
  fail({ message: "Late failure", preventDefault() {} });
  assert.deepEqual(failures, ["Worker failed"]);
  assert.equal(workers[0].terminated, true);
});

test("video publication defaults to the background clock and closes it on stop", async (t) => {
  const workers = workerHarness(t);
  let captures = 0;
  let publications = 0;
  const stop = startScreenPublisher({
    capture: async () => {
      captures++;
      return { content: "{}" };
    },
    publish: async () => publications++,
    fail: async (error) => assert.fail(error),
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(publications, 1);
  workers[0].reply(workers[0].sent[0].id);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(publications, 2);
  const queuedReply = workers[0].onmessage;
  const queuedId = workers[0].sent.at(-1).id;
  stop();
  queuedReply({ data: queuedId });
  assert.equal(captures, 2);
  assert.equal(workers[0].terminated, true);
});

test("publication failure closes its worker and propagates the terminal failure", async (t) => {
  const workers = workerHarness(t);
  const failures = [];
  startScreenPublisher({
    capture: async () => {
      throw new Error("Native capture failed");
    },
    publish: async () => assert.fail("Nothing may be published"),
    fail: async (error) => failures.push(error.message),
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(failures, ["Native capture failed"]);
  assert.equal(workers[0].terminated, true);
});
