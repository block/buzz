import assert from "node:assert/strict";
import test from "node:test";

import {
  USER_TIMING_SWEEP_INTERVAL_MS,
  startUserTimingSweep,
} from "./userTimingSweep.ts";

function makeTimers() {
  const intervals = [];
  return {
    intervals,
    setInterval: (callback, ms) => {
      intervals.push({ callback, ms, cleared: false });
      return intervals.length;
    },
    clearInterval: (id) => {
      intervals[id - 1].cleared = true;
    },
  };
}

test("clears measures on every tick until stopped", () => {
  const timers = makeTimers();
  let cleared = 0;
  const stop = startUserTimingSweep({
    enabled: true,
    performance: {
      clearMeasures: () => {
        cleared += 1;
      },
    },
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
  });

  assert.equal(timers.intervals.length, 1);
  assert.equal(timers.intervals[0].ms, USER_TIMING_SWEEP_INTERVAL_MS);
  timers.intervals[0].callback();
  timers.intervals[0].callback();
  assert.equal(cleared, 2);

  stop();
  assert.equal(timers.intervals[0].cleared, true);
});

test("is a no-op when disabled or when clearMeasures is unavailable", () => {
  const timers = makeTimers();
  startUserTimingSweep({
    enabled: false,
    performance: { clearMeasures: () => {} },
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
  });
  startUserTimingSweep({
    enabled: true,
    performance: {},
    setInterval: timers.setInterval,
    clearInterval: timers.clearInterval,
  });
  assert.equal(timers.intervals.length, 0);
});
