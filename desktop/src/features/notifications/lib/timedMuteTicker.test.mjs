import assert from "node:assert/strict";
import test from "node:test";

// The ticker reads window.setTimeout at call time, so a stub window is enough.
const timers = [];
globalThis.window = {
  clearTimeout: (id) => {
    const timer = timers.find((t) => t.id === id);
    if (timer) timer.cleared = true;
  },
  setTimeout: (fn, delay) => {
    timers.push({ cleared: false, delay, fn, id: timers.length + 1 });
    return timers.length;
  },
};

const {
  getTimedMuteVersion,
  resetTimedMuteTicker,
  scheduleTimedMuteRefresh,
  subscribeTimedMuteVersion,
} = await import("./timedMuteTicker.ts");

function pending() {
  return timers.filter((t) => !t.cleared);
}

test("scheduling the same expiry twice arms only one timer", () => {
  timers.length = 0;
  const expiry = Math.floor(Date.now() / 1_000) + 60;
  scheduleTimedMuteRefresh(expiry);
  scheduleTimedMuteRefresh(expiry);
  assert.equal(pending().length, 1);
  resetTimedMuteTicker();
});

test("a nearer expiry replaces the armed timer", () => {
  timers.length = 0;
  const now = Math.floor(Date.now() / 1_000);
  scheduleTimedMuteRefresh(now + 600);
  scheduleTimedMuteRefresh(now + 60);
  assert.equal(pending().length, 1);
  assert.ok(pending()[0].delay < 120_000);
  resetTimedMuteTicker();
});

test("null disarms the timer", () => {
  timers.length = 0;
  scheduleTimedMuteRefresh(Math.floor(Date.now() / 1_000) + 60);
  scheduleTimedMuteRefresh(null);
  assert.equal(pending().length, 0);
});

test("firing bumps the version and notifies subscribers", () => {
  timers.length = 0;
  let notified = 0;
  const unsubscribe = subscribeTimedMuteVersion(() => {
    notified += 1;
  });
  const before = getTimedMuteVersion();
  scheduleTimedMuteRefresh(Math.floor(Date.now() / 1_000) + 60);
  pending()[0].fn();
  assert.equal(getTimedMuteVersion(), before + 1);
  assert.equal(notified, 1);
  unsubscribe();
  resetTimedMuteTicker();
});

test("an already-past expiry arms an immediate timer rather than a negative delay", () => {
  timers.length = 0;
  scheduleTimedMuteRefresh(Math.floor(Date.now() / 1_000) - 3_600);
  assert.equal(pending().length, 1);
  assert.equal(pending()[0].delay, 0);
  resetTimedMuteTicker();
});

test("a far-future expiry is capped so setTimeout cannot overflow", () => {
  timers.length = 0;
  scheduleTimedMuteRefresh(Math.floor(Date.now() / 1_000) + 90 * 86_400);
  assert.equal(pending()[0].delay, 6 * 60 * 60 * 1_000);
  resetTimedMuteTicker();
});

test("reset drops subscribers so a later fire cannot reach them", () => {
  timers.length = 0;
  let notified = 0;
  subscribeTimedMuteVersion(() => {
    notified += 1;
  });
  scheduleTimedMuteRefresh(Math.floor(Date.now() / 1_000) + 60);
  const timer = pending()[0];
  resetTimedMuteTicker();
  timer.fn();
  assert.equal(notified, 0);
});
