/**
 * Unit tests for RelayAutoHealScheduler.
 *
 * Tests the rate-limited, deferred-heal state machine without React or DOM.
 * Covers: first recovery fires immediately, rate-limit suppression schedules
 * deferred heal, second recovery within window supersedes the deferred, heal
 * fires after deferred window expires, dispose cancels pending heal.
 */

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { RelayAutoHealScheduler } from "./useRelayAutoHeal.ts";

function makeScheduler({ minIntervalMs = 100, now: initialNow = 0 } = {}) {
  const heals = [];
  const timers = [];
  let timerSeq = 0;
  let nowMs = initialNow;

  const sched = new RelayAutoHealScheduler(
    () => heals.push(nowMs),
    minIntervalMs,
    mock.fn((fn, ms) => {
      const id = ++timerSeq;
      timers.push({ id, fn, ms });
      return id;
    }),
    mock.fn((id) => {
      const idx = timers.findIndex((t) => t.id === id);
      if (idx !== -1) timers.splice(idx, 1);
    }),
    () => nowMs,
  );

  return {
    sched,
    heals,
    timers,
    setNow: (ms) => {
      nowMs = ms;
    },
    fireTimer: (id) => timers.find((t) => t.id === id)?.fn(),
  };
}

// ── First recovery — fires immediately ───────────────────────────────────────

test("first recovery fires onHeal immediately", () => {
  const { sched, heals } = makeScheduler();

  sched.onTransition("reconnecting", "connected");

  assert.equal(heals.length, 1, "onHeal fired once");
});

test("non-recovery transitions are ignored", () => {
  const { sched, heals, timers } = makeScheduler();

  sched.onTransition("connected", "reconnecting");
  sched.onTransition("reconnecting", "stalled");
  sched.onTransition("connected", "connected");

  assert.equal(heals.length, 0, "no heals for non-recovery transitions");
  assert.equal(timers.length, 0, "no timers scheduled");
});

// ── Rate limiting ─────────────────────────────────────────────────────────────

test("second recovery within window is rate-limited and schedules deferred heal", () => {
  const { sched, heals, timers, setNow } = makeScheduler({
    minIntervalMs: 100,
  });

  setNow(0);
  sched.onTransition("reconnecting", "connected"); // fires immediately
  assert.equal(heals.length, 1);

  // Second recovery 50ms later — within the 100ms window.
  setNow(50);
  sched.onTransition("stalled", "connected");

  assert.equal(heals.length, 1, "onHeal not called again immediately");
  assert.equal(timers.length, 1, "deferred heal timer scheduled");
  assert.equal(
    timers[0].ms,
    50,
    "deferred fires after remaining window (100-50=50ms)",
  );
});

test("deferred heal fires onHeal when timer fires", () => {
  const { sched, heals, timers, setNow, fireTimer } = makeScheduler({
    minIntervalMs: 100,
  });

  setNow(0);
  sched.onTransition("reconnecting", "connected");

  setNow(50);
  sched.onTransition("stalled", "connected"); // rate-limited, timer id=1

  setNow(100);
  fireTimer(timers[0].id);

  assert.equal(heals.length, 2, "deferred onHeal fired after timer");
});

test("second recovery supersedes deferred — old timer cancelled, new one scheduled", () => {
  const { sched, heals, timers, setNow, fireTimer } = makeScheduler({
    minIntervalMs: 100,
  });

  setNow(0);
  sched.onTransition("reconnecting", "connected"); // immediate heal

  setNow(40);
  sched.onTransition("stalled", "connected"); // rate-limited → deferred at remaining=60ms
  const firstTimerId = timers[0].id;
  assert.equal(timers.length, 1);

  setNow(60);
  sched.onTransition("reconnecting", "connected"); // supersedes — cancel first, schedule new

  assert.equal(
    timers.findIndex((t) => t.id === firstTimerId),
    -1,
    "first timer cancelled",
  );
  assert.equal(timers.length, 1, "new deferred timer scheduled");
  assert.equal(
    timers[0].ms,
    40,
    "new deferred fires after remaining window (100-60=40ms)",
  );

  setNow(100);
  fireTimer(timers[0].id);

  assert.equal(heals.length, 2, "exactly two heals total (initial + deferred)");
});

// ── Dispose ───────────────────────────────────────────────────────────────────

test("dispose cancels pending deferred heal", () => {
  const { sched, heals, timers, setNow } = makeScheduler({
    minIntervalMs: 100,
  });

  setNow(0);
  sched.onTransition("reconnecting", "connected");

  setNow(50);
  sched.onTransition("stalled", "connected"); // deferred scheduled

  sched.dispose();
  assert.equal(timers.length, 0, "timer cancelled on dispose");

  // Firing the timer after dispose should be a no-op (timer is gone).
  // The heals array should still be 1 (only the initial immediate heal).
  assert.equal(heals.length, 1, "no additional heals after dispose");
});

test("dispose with no pending timer is a no-op", () => {
  const { sched } = makeScheduler();
  assert.doesNotThrow(() => sched.dispose());
});
