import assert from "node:assert/strict";
import test from "node:test";

import {
  isMsgContextKey,
  isPlausibleReadMarker,
  isThreadContextKey,
  MAX_READ_MARKER_SKEW_SECONDS,
  maxReadAt,
  msgContextKey,
  sanitizeContexts,
} from "./readStateFormat.ts";

const EVENT_ID = "a".repeat(64);

test("maxReadAt_usesNewestNonNullMarker", () => {
  assert.equal(maxReadAt(null, 10, 5, null, 30), 30);
});

test("maxReadAt_allNull_returnsNull", () => {
  assert.equal(maxReadAt(null, null), null);
});

test("msgContextKey_prefixesId_returnsMsgKey", () => {
  assert.equal(msgContextKey(EVENT_ID), `msg:${EVENT_ID}`);
});

test("isMsgContextKey_wellFormedKey_returnsTrue", () => {
  assert.equal(isMsgContextKey(`msg:${EVENT_ID}`), true);
});

test("isThreadContextKey_wellFormedKey_returnsTrue", () => {
  assert.equal(isThreadContextKey(`thread:${EVENT_ID}`), true);
});

test("isMsgContextKey_threadKey_returnsFalse", () => {
  assert.equal(isMsgContextKey(`thread:${EVENT_ID}`), false);
});

test("isMsgContextKey_channelKey_returnsFalse", () => {
  assert.equal(isMsgContextKey("channel-1"), false);
});

test("isMsgContextKey_emptyId_returnsFalse", () => {
  assert.equal(isMsgContextKey("msg:"), false);
});

test("isMsgContextKey_shortId_returnsFalse", () => {
  assert.equal(isMsgContextKey("msg:abc123"), false);
});

test("isMsgContextKey_msgPrefixWrappingThreadKey_returnsFalse", () => {
  // A thread key accidentally re-prefixed must not pass as a message key.
  assert.equal(isMsgContextKey(`msg:thread:${EVENT_ID}`), false);
});

test("isThreadContextKey_shortId_returnsFalse", () => {
  assert.equal(isThreadContextKey("thread:abc123"), false);
});

test("msgContextKey_output_roundTripsThroughValidator", () => {
  assert.equal(isMsgContextKey(msgContextKey(EVENT_ID)), true);
});

// --- Synced read state carries the same skew policy -----------------------
//
// A NIP-RS blob may have been published by another desktop that predates the
// policy. Admitting a year-ahead context there would poison this device too,
// and monotonic merging means it would never expire.

test("sanitizeContexts_dropsAnImplausiblyFutureMarker", () => {
  const now = 1_780_000_000;

  const result = sanitizeContexts(
    {
      "channel-real": now - 3_600,
      "channel-skewed": now + 30,
      "channel-poisoned": now + 365 * 24 * 60 * 60,
    },
    now,
  );

  assert.deepEqual(result, {
    "channel-real": now - 3_600,
    "channel-skewed": now + 30,
  });
});

test("sanitizeContexts_stillDropsTheOldMalformedShapes", () => {
  const now = 1_780_000_000;

  assert.deepEqual(
    sanitizeContexts(
      { a: "12", b: 1.5, c: -1, d: 4_294_967_296, e: now - 1 },
      now,
    ),
    { e: now - 1 },
  );
});

test("isPlausibleReadMarker_boundaryIsInclusive", () => {
  const now = 1_780_000_000;
  assert.equal(
    isPlausibleReadMarker(now + MAX_READ_MARKER_SKEW_SECONDS, now),
    true,
  );
  assert.equal(
    isPlausibleReadMarker(now + MAX_READ_MARKER_SKEW_SECONDS + 1, now),
    false,
  );
});
