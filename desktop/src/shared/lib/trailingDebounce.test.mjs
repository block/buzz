import assert from "node:assert/strict";
import test from "node:test";

import { createTrailingDebounce } from "./trailingDebounce.ts";

// Deterministic timer host: timers fire only when we call advance(), so no real
// time elapses. Mirrors the injectable-timer style of relayStallWatchdog.
function makeHost() {
  let nextId = 1;
  const pending = new Map();
  return {
    host: {
      setTimeout: (handler, ms) => {
        const id = nextId++;
        pending.set(id, { handler, remaining: ms });
        return id;
      },
      clearTimeout: (id) => {
        pending.delete(id);
      },
    },
    advance: (ms) => {
      for (const [id, t] of [...pending]) {
        t.remaining -= ms;
        if (t.remaining <= 0) {
          pending.delete(id);
          t.handler();
        }
      }
    },
    pendingCount: () => pending.size,
  };
}

test("coalesces a burst into a single trailing call", () => {
  const { host, advance } = makeHost();
  let calls = 0;
  const d = createTrailingDebounce(() => calls++, 500, host);

  d.trigger();
  advance(100);
  d.trigger(); // restarts the window
  advance(100);
  d.trigger(); // restarts again
  assert.equal(calls, 0, "must not fire during the burst");

  advance(500); // quiet period elapses after the last trigger
  assert.equal(calls, 1, "fires exactly once after quiet");
});

test("restarts the quiet window on every trigger", () => {
  const { host, advance } = makeHost();
  let calls = 0;
  const d = createTrailingDebounce(() => calls++, 500, host);

  d.trigger();
  advance(499);
  d.trigger(); // reset just before firing
  advance(499);
  assert.equal(calls, 0, "reset prevents the earlier timer from firing");
  advance(1);
  assert.equal(calls, 1);
});

test("cancel() drops the pending call", () => {
  const { host, advance, pendingCount } = makeHost();
  let calls = 0;
  const d = createTrailingDebounce(() => calls++, 500, host);

  d.trigger();
  d.cancel();
  assert.equal(pendingCount(), 0, "no timer left pending");
  advance(500);
  assert.equal(calls, 0, "cancelled action never runs");
});

test("a later trigger after firing schedules a fresh call", () => {
  const { host, advance } = makeHost();
  let calls = 0;
  const d = createTrailingDebounce(() => calls++, 500, host);

  d.trigger();
  advance(500);
  assert.equal(calls, 1);

  d.trigger();
  advance(500);
  assert.equal(calls, 2);
});
