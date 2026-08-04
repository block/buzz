import assert from "node:assert/strict";
import test from "node:test";

import {
  RESUME_TRIGGER_MIN_INTERVAL_MS,
  shouldTriggerResumeReconnect,
} from "./relayResumeTriggerPolicy.ts";

const base = Object.freeze({
  connectionState: "reconnecting",
  lastAttemptAt: 0,
  now: RESUME_TRIGGER_MIN_INTERVAL_MS + 1,
});

test("reconnecting + interval elapsed triggers", () => {
  assert.equal(shouldTriggerResumeReconnect({ ...base }), true);
});

test("stalled + interval elapsed triggers", () => {
  assert.equal(
    shouldTriggerResumeReconnect({ ...base, connectionState: "stalled" }),
    true,
  );
});

test("healthy and terminal states never trigger", () => {
  for (const connectionState of [
    "idle",
    "connecting",
    "connected",
    "disconnected",
  ]) {
    assert.equal(
      shouldTriggerResumeReconnect({ ...base, connectionState }),
      false,
      connectionState,
    );
  }
});

test("burst of triggers within the rate window fires once", () => {
  assert.equal(
    shouldTriggerResumeReconnect({
      ...base,
      lastAttemptAt: 1_000,
      now: 1_000 + RESUME_TRIGGER_MIN_INTERVAL_MS - 1,
    }),
    false,
  );
  assert.equal(
    shouldTriggerResumeReconnect({
      ...base,
      lastAttemptAt: 1_000,
      now: 1_000 + RESUME_TRIGGER_MIN_INTERVAL_MS,
    }),
    true,
  );
});

test("custom interval override is honoured", () => {
  assert.equal(
    shouldTriggerResumeReconnect({
      ...base,
      lastAttemptAt: 0,
      now: 10,
      minIntervalMs: 5,
    }),
    true,
  );
});
