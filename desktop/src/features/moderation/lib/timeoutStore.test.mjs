/**
 * Unit tests for timeout-store behavioral invariants (item 7).
 *
 * These pin the store semantics that useTimeoutState builds on. The reactive
 * hook itself is an integration concern; these tests cover the pure-logic layer:
 * record, clear, snapshot, and the interaction between the store's raw `active`
 * flag and `isTimeoutActive`'s expiry check.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { isTimeoutActive, formatTimeoutRemaining } from "./timeout.ts";
import {
  recordTimeoutFromRejection,
  clearTimeoutState,
  getTimeoutSnapshot,
} from "./timeoutStore.ts";

// Helper: bring the store to a clean INACTIVE state.
function reset() {
  clearTimeoutState();
}

test("expired-restriction-inactive: a known past expiry is INACTIVE per isTimeoutActive", () => {
  reset();
  const now = 1_000_000_000_000;
  const pastUnixSec = Math.floor((now - 60_000) / 1000);
  recordTimeoutFromRejection(
    `restricted: you are timed out until ${pastUnixSec}`,
  );
  const snap = getTimeoutSnapshot();
  assert.equal(snap.active, true, "store records active flag from rejection");
  assert.notEqual(snap.expiresAtMs, null, "store records non-null expiry");
  // The expiry has passed — isTimeoutActive must report false.
  assert.equal(
    isTimeoutActive(snap.expiresAtMs, now),
    false,
    "isTimeoutActive must report false for a past expiry",
  );
  reset();
});

test("null-expiry-fails-closed: no-timestamp timeout records active with null expiry, isTimeoutActive stays true", () => {
  reset();
  recordTimeoutFromRejection("restricted: you are timed out until unparseable");
  const snap = getTimeoutSnapshot();
  assert.equal(snap.active, true, "store must be active");
  assert.equal(
    snap.expiresAtMs,
    null,
    "expiresAtMs must be null for unparseable timestamp",
  );
  assert.equal(
    isTimeoutActive(null, Date.now()),
    true,
    "isTimeoutActive must stay true for null expiry (fail-closed)",
  );
  reset();
});

test("clear-transitions-to-inactive: clearTimeoutState resolves any active timeout", () => {
  reset();
  const futureUnixSec = Math.floor(Date.now() / 1000) + 3600;
  recordTimeoutFromRejection(
    `restricted: you are timed out until ${futureUnixSec}`,
  );
  assert.equal(getTimeoutSnapshot().active, true, "active before clear");
  clearTimeoutState();
  const snap = getTimeoutSnapshot();
  assert.equal(snap.active, false, "inactive after clear");
  assert.equal(snap.expiresAtMs, null, "expiresAtMs null after clear");
});

test("non-timeout-rejection-ignored: an unrelated message does not set active", () => {
  reset();
  const result = recordTimeoutFromRejection(
    "blocked: you are banned from this community",
  );
  assert.equal(result, false, "must return false for non-timeout message");
  assert.equal(
    getTimeoutSnapshot().active,
    false,
    "store must remain inactive",
  );
});

test("expired-overlay-guard: formatTimeoutRemaining returns null for past/boundary expiry — never an empty string", () => {
  // Regression guard for the empty-overlay bug: when the TTL expires,
  // ComposerTimeoutBanner must NOT render a blank countdown string. A null
  // return means it falls back to "You're timed out..." (correct), never "".
  const now = 1_000_000_000_000;

  // Exactly at expiry (totalSeconds = 0 → null).
  assert.equal(
    formatTimeoutRemaining(now, now),
    null,
    "formatTimeoutRemaining must return null at exact expiry boundary",
  );

  // Past expiry.
  assert.equal(
    formatTimeoutRemaining(now - 5_000, now),
    null,
    "formatTimeoutRemaining must return null for a past expiry",
  );

  // Unknown expiry.
  assert.equal(
    formatTimeoutRemaining(null, now),
    null,
    "formatTimeoutRemaining must return null for unknown expiry",
  );

  // Future expiry returns a non-null, non-empty string.
  const s = formatTimeoutRemaining(now + 90_000, now);
  assert.ok(
    s !== null && s !== "",
    `formatTimeoutRemaining must return a non-empty string for a future expiry; got: ${s}`,
  );
});
