import assert from "node:assert/strict";
import test from "node:test";

import {
  hasObservableTimeout,
  isTimedOut,
  parseRestrictionTimestampMs,
} from "./restrictionState.ts";

test("parses an RFC3339 string to epoch ms", () => {
  assert.equal(
    parseRestrictionTimestampMs("2026-07-07T22:00:00Z"),
    Date.parse("2026-07-07T22:00:00Z"),
  );
});

test("treats a legacy number as unix seconds", () => {
  assert.equal(parseRestrictionTimestampMs(1751920000), 1751920000 * 1000);
});

test("returns null for a null value", () => {
  assert.equal(parseRestrictionTimestampMs(null), null);
});

test("returns null for an unparseable string (fails closed)", () => {
  assert.equal(parseRestrictionTimestampMs("not-a-date"), null);
});

test("isTimedOut is true for a future muted-until", () => {
  const now = 1_000_000_000_000;
  assert.equal(isTimedOut(new Date(now + 60_000).toISOString(), now), true);
});

test("isTimedOut is false for a past muted-until", () => {
  const now = 1_000_000_000_000;
  assert.equal(isTimedOut(new Date(now - 60_000).toISOString(), now), false);
});

test("isTimedOut is false for an absent muted-until (fail closed to not-timed-out)", () => {
  assert.equal(isTimedOut(null), false);
});

test("isTimedOut is false for an unparseable muted-until", () => {
  assert.equal(isTimedOut("garbage"), false);
});

const NOW = 1_000_000_000_000;
const future = () => new Date(NOW + 60_000).toISOString();
const past = () => new Date(NOW - 60_000).toISOString();

test("hasObservableTimeout is false for no restrictions", () => {
  assert.equal(hasObservableTimeout([], NOW), false);
});

test("hasObservableTimeout is true for a future mute", () => {
  assert.equal(hasObservableTimeout([{ mutedUntil: future() }], NOW), true);
});

test("hasObservableTimeout is false for a past mute", () => {
  assert.equal(hasObservableTimeout([{ mutedUntil: past() }], NOW), false);
});

test("hasObservableTimeout is false for a null mute (ban-only)", () => {
  assert.equal(hasObservableTimeout([{ mutedUntil: null }], NOW), false);
});

test("hasObservableTimeout is true when any restriction has a future mute", () => {
  assert.equal(
    hasObservableTimeout(
      [{ mutedUntil: past() }, { mutedUntil: null }, { mutedUntil: future() }],
      NOW,
    ),
    true,
  );
});
