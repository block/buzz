import assert from "node:assert/strict";
import test from "node:test";
import { scheduleSettleGatedAutoSubmit } from "./messageComposerAutoSubmit.ts";

function makeFakeTimers() {
  const pending = new Map();
  let nextId = 1;
  return {
    set(fn) {
      const id = nextId++;
      pending.set(id, fn);
      return id;
    },
    clear(id) {
      pending.delete(id);
    },
    tick() {
      const [id, fn] = pending.entries().next().value ?? [];
      if (id === undefined) return false;
      pending.delete(id);
      fn();
      return true;
    },
    pendingCount() {
      return pending.size;
    },
  };
}

test("submits a restored draft exactly once on the next task", () => {
  const timers = makeFakeTimers();
  let submits = 0;
  scheduleSettleGatedAutoSubmit({
    submit: () => submits++,
    timers,
  });
  assert.equal(submits, 0);
  timers.tick();
  assert.equal(submits, 1);
  assert.equal(timers.pendingCount(), 0);
});

test("cleanup before the next task cancels the submit", () => {
  const timers = makeFakeTimers();
  let submits = 0;
  const cleanup = scheduleSettleGatedAutoSubmit({
    submit: () => submits++,
    timers,
  });
  cleanup();
  assert.equal(timers.pendingCount(), 0);
  assert.equal(submits, 0);
});
