import assert from "node:assert/strict";
import test from "node:test";

import { createHoverIntent } from "./useHoverIntent.ts";

function fakeTimers() {
  const timers = new Map();
  let nextId = 1;
  return {
    setTimeout: (fn, _ms) => {
      const id = nextId++;
      timers.set(id, fn);
      return id;
    },
    clearTimeout: (id) => timers.delete(id),
    fire: () => {
      for (const [id, fn] of [...timers]) {
        timers.delete(id);
        fn();
      }
    },
    pending: () => timers.size,
  };
}

test("fires the callback only after the dwell elapses", () => {
  const timers = fakeTimers();
  let fired = 0;
  const intent = createHoverIntent(() => fired++, timers);

  intent.start();
  assert.equal(fired, 0);
  timers.fire();
  assert.equal(fired, 1);
});

test("leaving before the dwell cancels the callback", () => {
  const timers = fakeTimers();
  let fired = 0;
  const intent = createHoverIntent(() => fired++, timers);

  intent.start();
  intent.cancel();
  timers.fire();
  assert.equal(fired, 0);
  assert.equal(timers.pending(), 0);
});

test("re-entering restarts the dwell without stacking timers", () => {
  const timers = fakeTimers();
  let fired = 0;
  const intent = createHoverIntent(() => fired++, timers);

  intent.start();
  intent.start();
  assert.equal(timers.pending(), 1);
  timers.fire();
  assert.equal(fired, 1);
});
