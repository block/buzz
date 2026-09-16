import assert from "node:assert/strict";
import test from "node:test";

import {
  AuthOkTracker,
  MAX_CONSECUTIVE_AUTH_REJECTIONS,
} from "./relayAuthPolicy.ts";

test("success resolves authenticated and resets the streak", () => {
  const tracker = new AuthOkTracker();
  tracker.record(false, "auth-required: verification failed");
  tracker.record(false, "auth-required: verification failed");
  assert.equal(tracker.record(true, ""), "authenticated");
  // Streak reset: the next rejection starts a fresh count.
  assert.equal(
    tracker.record(false, "auth-required: verification failed"),
    "retry",
  );
});

test("already-authenticated rejection is treated as authenticated (duplicate-AUTH race)", () => {
  const tracker = new AuthOkTracker();
  assert.equal(
    tracker.record(false, "auth-required: already authenticated"),
    "authenticated",
  );
});

test("restricted rejections latch terminal immediately", () => {
  for (const message of [
    "restricted: not a relay member",
    "restricted: you are banned",
  ]) {
    assert.equal(new AuthOkTracker().record(false, message), "terminal");
  }
});

test("the relay's actual ban string latches terminal immediately", () => {
  // Exact string emitted by crates/buzz-relay/src/handlers/auth.rs at the
  // ban seam. A known-permanent ban must never enter the retry loop.
  assert.equal(
    new AuthOkTracker().record(
      false,
      "blocked: you are banned from this community",
    ),
    "terminal",
  );
});

test("verification failures retry with backoff (clock skew, DB fail-closed)", () => {
  const tracker = new AuthOkTracker();
  assert.equal(
    tracker.record(false, "auth-required: verification failed"),
    "retry",
  );
});

test("unknown rejection reasons retry rather than latch", () => {
  assert.equal(new AuthOkTracker().record(false, ""), "retry");
  assert.equal(new AuthOkTracker().record(false, "error: internal"), "retry");
});

test("consecutive rejections latch terminal at the cap", () => {
  const tracker = new AuthOkTracker();
  let decision = "retry";
  for (let i = 0; i < MAX_CONSECUTIVE_AUTH_REJECTIONS; i++) {
    decision = tracker.record(false, "auth-required: verification failed");
  }
  assert.equal(decision, "terminal");
});

test("reset grants a fresh retry streak after explicit re-engagement", () => {
  const tracker = new AuthOkTracker();
  for (let i = 0; i < MAX_CONSECUTIVE_AUTH_REJECTIONS; i++) {
    tracker.record(false, "auth-required: verification failed");
  }
  tracker.reset();
  assert.equal(
    tracker.record(false, "auth-required: verification failed"),
    "retry",
  );
});

test("already-authenticated wins even past the cap (session is usable)", () => {
  const tracker = new AuthOkTracker();
  for (let i = 0; i < MAX_CONSECUTIVE_AUTH_REJECTIONS + 1; i++) {
    tracker.record(false, "auth-required: verification failed");
  }
  assert.equal(
    tracker.record(false, "auth-required: already authenticated"),
    "authenticated",
  );
});
