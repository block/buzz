import assert from "node:assert/strict";
import test from "node:test";

import {
  formatTimeoutRemaining,
  isTimeoutActive,
  parseTimeoutRejection,
} from "./timeout.ts";

test("parses a well-formed timeout rejection to epoch ms", () => {
  const result = parseTimeoutRejection(
    "restricted: you are timed out until 1751920000",
  );
  assert.deepEqual(result, { expiresAtMs: 1751920000 * 1000 });
});

test("tolerates surrounding whitespace", () => {
  const result = parseTimeoutRejection(
    "  restricted: you are timed out until 1751920000  ",
  );
  assert.deepEqual(result, { expiresAtMs: 1751920000 * 1000 });
});

test("returns null for a non-timeout rejection", () => {
  assert.equal(
    parseTimeoutRejection("blocked: you are banned from this community"),
    null,
  );
  assert.equal(parseTimeoutRejection("restricted: not a channel member"), null);
  assert.equal(parseTimeoutRejection(""), null);
  assert.equal(parseTimeoutRejection(null), null);
  assert.equal(parseTimeoutRejection(undefined), null);
});

test("timeout with unparseable timestamp still signals timed-out", () => {
  assert.deepEqual(
    parseTimeoutRejection("restricted: you are timed out until soon"),
    { expiresAtMs: null },
  );
  assert.deepEqual(
    parseTimeoutRejection("restricted: you are timed out until "),
    { expiresAtMs: null },
  );
  assert.deepEqual(
    parseTimeoutRejection("restricted: you are timed out until -5"),
    { expiresAtMs: null },
  );
});

test("isTimeoutActive: future expiry active, past expiry inactive", () => {
  const now = 1_000_000_000_000;
  assert.equal(isTimeoutActive(now + 5000, now), true);
  assert.equal(isTimeoutActive(now - 5000, now), false);
});

test("isTimeoutActive: unknown expiry fails closed (active)", () => {
  assert.equal(isTimeoutActive(null, 1_000_000_000_000), true);
});

test("formatTimeoutRemaining: hours, minutes, seconds tiers", () => {
  const now = 1_000_000_000_000;
  assert.equal(
    formatTimeoutRemaining(now + (2 * 3600 + 5 * 60) * 1000, now),
    "2h 5m",
  );
  assert.equal(
    formatTimeoutRemaining(now + (3 * 60 + 20) * 1000, now),
    "3m 20s",
  );
  assert.equal(formatTimeoutRemaining(now + 12 * 1000, now), "12s");
});

test("formatTimeoutRemaining: null when unknown or elapsed", () => {
  const now = 1_000_000_000_000;
  assert.equal(formatTimeoutRemaining(null, now), null);
  assert.equal(formatTimeoutRemaining(now - 1000, now), null);
  assert.equal(formatTimeoutRemaining(now, now), null);
});
