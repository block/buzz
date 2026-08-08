import assert from "node:assert/strict";
import test from "node:test";

import { createCompletionPollScheduler } from "./completionPollScheduler.ts";

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function fakeTimers() {
  let nextHandle = 1;
  const callbacks = new Map();
  return {
    scheduleTimer(callback) {
      const handle = nextHandle++;
      callbacks.set(handle, callback);
      return handle;
    },
    cancelTimer(handle) {
      callbacks.delete(handle);
    },
    runNext() {
      const entry = callbacks.entries().next().value;
      assert.ok(entry, "expected a scheduled timer");
      const [handle, callback] = entry;
      callbacks.delete(handle);
      callback();
    },
    pending() {
      return callbacks.size;
    },
  };
}

test("repeated timer ticks while a poll is unresolved produce exactly one call", async () => {
  const gate = deferred();
  const timers = fakeTimers();
  let calls = 0;
  const scheduler = createCompletionPollScheduler({
    poll: async () => {
      calls += 1;
      await gate.promise;
    },
    delayMs: 5_000,
    ...timers,
  });

  await Promise.resolve();
  const repeated = Array.from({ length: 100 }, () => scheduler.trigger());
  assert.equal(calls, 1);
  assert.equal(timers.pending(), 0);

  gate.resolve();
  await Promise.all(repeated);
  assert.equal(timers.pending(), 1);
  scheduler.stop();
});

test("the next poll starts only after completion and the delay", async () => {
  const timers = fakeTimers();
  let calls = 0;
  const scheduler = createCompletionPollScheduler({
    poll: async () => {
      calls += 1;
    },
    delayMs: 5_000,
    ...timers,
  });

  await scheduler.trigger();
  assert.equal(calls, 1);
  assert.equal(timers.pending(), 1);

  timers.runNext();
  await scheduler.trigger();
  assert.equal(calls, 2);
  assert.equal(timers.pending(), 1);
  scheduler.stop();
});

test("cleanup cancels future polls and suppresses rescheduling", async () => {
  const gate = deferred();
  const timers = fakeTimers();
  let calls = 0;
  const scheduler = createCompletionPollScheduler({
    poll: async () => {
      calls += 1;
      await gate.promise;
    },
    delayMs: 5_000,
    ...timers,
  });

  await Promise.resolve();
  scheduler.stop();
  gate.resolve();
  await scheduler.trigger().catch(() => {});

  assert.equal(calls, 1);
  assert.equal(timers.pending(), 0);
  await scheduler.trigger();
  assert.equal(calls, 1);
});

test("a failed automatic poll still schedules exactly one retry", async () => {
  const timers = fakeTimers();
  let calls = 0;
  const scheduler = createCompletionPollScheduler({
    poll: async () => {
      calls += 1;
      if (calls === 1) throw new Error("temporary failure");
    },
    delayMs: 5_000,
    ...timers,
  });

  await scheduler.trigger().catch(() => {});
  assert.equal(calls, 1);
  assert.equal(timers.pending(), 1);
  timers.runNext();
  await scheduler.trigger();
  assert.equal(calls, 2);
  assert.equal(timers.pending(), 1);
  scheduler.stop();
});
