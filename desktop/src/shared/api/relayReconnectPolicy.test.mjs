import assert from "node:assert/strict";
import test from "node:test";

import {
  isServiceRestartClose,
  isWebSocketClose,
  shouldRefuseConnect,
  shouldScheduleReconnect,
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

test("shouldRefuseConnect mirrors terminal", () => {
  assert.equal(shouldRefuseConnect({ terminal: false }), false);
  assert.equal(shouldRefuseConnect({ terminal: true }), true);
});

test("only a close frame with code 1012 is a service restart", () => {
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
