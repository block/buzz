import assert from "node:assert/strict";
import test from "node:test";

import {
  isServiceRestartClose,
  RelayReconnectBackoff,
  SERVICE_RESTART_RECONNECT_SPREAD_MS,
  isWebSocketClose,
  shouldRefuseConnect,
  shouldScheduleReconnect,
  shouldWaitForScheduledReconnect,
} from "./relayReconnectPolicy.ts";

// The "happy" baseline that *should* schedule a reconnect: not terminal,
// no pending timer, no live socket, and at least something keeping the
// session alive (a live subscription, in this case).
const baseline = Object.freeze({
  terminal: false,
  hasPendingReconnect: false,
  hasLiveSocket: false,
  keepAliveRequested: false,
  hasLiveSubscriptions: true,
});

test("baseline scenario schedules a reconnect", () => {
  assert.equal(shouldScheduleReconnect({ ...baseline }), true);
});

test("terminal session refuses to schedule (Max's auth-rejection scenario)", () => {
  assert.equal(shouldScheduleReconnect({ ...baseline, terminal: true }), false);
});

test("terminal beats every other reason to reconnect", () => {
  // Even with every "yes please reconnect" predicate flipped on, terminal
  // wins. This is the critical guarantee against the reconnect timer's
  // catch handler resurrecting a dead session.
  assert.equal(
    shouldScheduleReconnect({
      terminal: true,
      hasPendingReconnect: false,
      hasLiveSocket: false,
      keepAliveRequested: true,
      hasLiveSubscriptions: true,
    }),
    false,
  );
});

test("pending reconnect timer suppresses scheduling another", () => {
  assert.equal(
    shouldScheduleReconnect({ ...baseline, hasPendingReconnect: true }),
    false,
  );
});

test("live socket suppresses scheduling", () => {
  assert.equal(
    shouldScheduleReconnect({ ...baseline, hasLiveSocket: true }),
    false,
  );
});

test("no live subs and no keep-alive → don't keep an idle socket up", () => {
  assert.equal(
    shouldScheduleReconnect({
      ...baseline,
      hasLiveSubscriptions: false,
      keepAliveRequested: false,
    }),
    false,
  );
});

test("keep-alive alone is enough to schedule", () => {
  assert.equal(
    shouldScheduleReconnect({
      ...baseline,
      hasLiveSubscriptions: false,
      keepAliveRequested: true,
    }),
    true,
  );
});

test("ordinary operations wait for a scheduled reconnect instead of bypassing backoff", () => {
  assert.equal(
    shouldWaitForScheduledReconnect({ hasPendingReconnect: true }),
    true,
  );
  assert.equal(
    shouldWaitForScheduledReconnect({ hasPendingReconnect: false }),
    false,
  );
});

test("shouldRefuseConnect mirrors terminal", () => {
  assert.equal(shouldRefuseConnect({ terminal: false }), false);
  assert.equal(shouldRefuseConnect({ terminal: true }), true);
});

test("only a close frame with code 1012 or 1001 is a service restart", () => {
  assert.equal(
    isServiceRestartClose({ type: "Close", data: { code: 1001 } }),
    true,
  );
  assert.equal(isWebSocketClose({ type: "Close" }), true);
  assert.equal(isWebSocketClose({ type: "Error" }), false);
  assert.equal(
    isServiceRestartClose({
      type: "Close",
      data: { code: 1012, reason: "relay restarting" },
    }),
    true,
  );
  assert.equal(
    isServiceRestartClose({ type: "Close", data: { code: 1000 } }),
    false,
  );
  assert.equal(isServiceRestartClose({ type: "Close" }), false);
  assert.equal(
    isServiceRestartClose({ type: "Error", data: { code: 1012 } }),
    false,
  );
  assert.equal(isServiceRestartClose(null), false);
});

test("service-restart close spreads the first reconnect uniformly over [0, 10s]", () => {
  assert.equal(SERVICE_RESTART_RECONNECT_SPREAD_MS, 10_000);
  for (const [random, expected] of [
    [0, 0],
    [0.25, 2_500],
    [0.5, 5_000],
    [0.999, 9_990],
  ]) {
    // A freshly reset backoff (1s) must not pin the delay near 1s.
    const backoff = new RelayReconnectBackoff(1_000, 30_000, () => random);
    assert.equal(backoff.next(true), expected);
  }
});

test("service-restart spreading does not reset the escalated backoff", () => {
  const backoff = new RelayReconnectBackoff(1_000, 30_000, () => 0.5);
  backoff.next();
  backoff.next();
  backoff.next();
  assert.equal(backoff.currentMs, 8_000);
  backoff.next(true);
  assert.equal(backoff.currentMs, 16_000, "bounced client keeps backing off");
  assert.equal(backoff.next(), 16_000, "next attempt uses the escalated base");
});

test("ordinary reconnect jitter is symmetric +/-25% and capped", () => {
  const at = (random, start = 0) => {
    const backoff = new RelayReconnectBackoff(1_000, 30_000, () => random);
    for (let i = 0; i < start; i++) backoff.next();
    return backoff.next();
  };
  assert.equal(at(0), 750);
  assert.equal(at(0.5), 1_000);
  assert.ok(Math.abs(at(0.999) - 1_249.5) < 1e-6);
  // At the cap the jittered delay never exceeds it.
  assert.equal(at(0.999, 10), 30_000);
  assert.equal(at(0, 10), 22_500);
});

test("backoff doubles per attempt up to the cap and only reset() returns it to base", () => {
  const backoff = new RelayReconnectBackoff(1_000, 30_000, () => 0.5);
  const delays = Array.from({ length: 7 }, () => backoff.next());
  assert.deepEqual(
    delays,
    [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000],
  );
  backoff.reset();
  assert.equal(backoff.next(), 1_000);
});

test("only a connection that stays up resets the backoff; a drop before then cancels it", () => {
  const timers = new Map();
  let nextId = 1;
  const originalWindow = globalThis.window;
  globalThis.window = {
    setTimeout(fn, ms) {
      timers.set(nextId, { fn, ms });
      return nextId++;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
  };
  try {
    const backoff = new RelayReconnectBackoff(1_000, 30_000, () => 0.5);
    for (let i = 0; i < 3; i++) backoff.next();
    backoff.resetWhenStable(60_000);
    assert.equal(
      backoff.currentMs,
      8_000,
      "opening a socket alone resets nothing",
    );
    backoff.cancelStableReset(); // dropped before it was stable
    assert.equal(timers.size, 0);
    assert.equal(backoff.currentMs, 8_000);

    backoff.resetWhenStable(60_000);
    backoff.resetWhenStable(60_000); // re-arming replaces, never stacks
    assert.equal(timers.size, 1);
    const [[id, timer]] = [...timers];
    assert.equal(timer.ms, 60_000);
    timers.delete(id);
    timer.fn();
    assert.equal(backoff.currentMs, 1_000, "stable for 60s: back to base");

    backoff.next();
    backoff.resetWhenStable(60_000);
    backoff.reset();
    assert.equal(timers.size, 0, "reset() also drops a pending stable reset");
  } finally {
    globalThis.window = originalWindow;
  }
});
