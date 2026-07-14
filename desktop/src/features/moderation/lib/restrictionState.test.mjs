import assert from "node:assert/strict";
import test from "node:test";

import { isTimedOut, parseRestrictionTimestampMs } from "./restrictionState.ts";

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
