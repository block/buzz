import assert from "node:assert/strict";
import test from "node:test";

import {
  OversizedFrameError,
  assertWithinFrameLimit,
  isOversizedFrameError,
  parseFrameTooLargeNotice,
  relayFrameByteLength,
} from "./relayFrameLimit.ts";

// ── parseFrameTooLargeNotice ─────────────────────────────────────────────────

test("parseFrameTooLargeNotice: parses valid notice", () => {
  const result = parseFrameTooLargeNotice(
    "frame too large (65536 bytes, limit 32768)",
  );
  assert.deepEqual(result, { bytes: 65536, limit: 32768 });
});

test("parseFrameTooLargeNotice: returns null for unrelated notices", () => {
  assert.equal(parseFrameTooLargeNotice("rate-limited: slow down"), null);
  assert.equal(parseFrameTooLargeNotice(""), null);
  assert.equal(parseFrameTooLargeNotice("frame too large"), null);
});

test("parseFrameTooLargeNotice: matches embedded in longer string", () => {
  const result = parseFrameTooLargeNotice(
    "error: frame too large (100 bytes, limit 50)",
  );
  assert.deepEqual(result, { bytes: 100, limit: 50 });
});

// ── relayFrameByteLength ─────────────────────────────────────────────────────

test("relayFrameByteLength: ASCII payload gives same byte and char count", () => {
  const payload = ["EVENT", { id: "abc" }];
  const json = JSON.stringify(payload);
  assert.equal(relayFrameByteLength(payload), json.length);
});

test("relayFrameByteLength: multi-byte UTF-8 chars counted correctly", () => {
  // "€" is 3 bytes in UTF-8 but 1 char
  const payload = ["NOTE", "€"];
  const bytes = relayFrameByteLength(payload);
  assert.ok(bytes > JSON.stringify(payload).length, "UTF-8 bytes > char count");
});

// ── assertWithinFrameLimit ───────────────────────────────────────────────────

test("assertWithinFrameLimit: does not throw when within limit", () => {
  const payload = ["REQ", "sub1", {}];
  assert.doesNotThrow(() => assertWithinFrameLimit(payload, 1_000_000));
});

test("assertWithinFrameLimit: throws OversizedFrameError when over limit", () => {
  const payload = ["EVENT", { content: "x".repeat(100) }];
  assert.throws(() => assertWithinFrameLimit(payload, 10), OversizedFrameError);
});

test("assertWithinFrameLimit: OversizedFrameError carries bytes and limit", () => {
  const payload = ["NOTE", "hello"];
  const bytes = relayFrameByteLength(payload);
  const limit = 1;
  try {
    assertWithinFrameLimit(payload, limit);
    assert.fail("expected OversizedFrameError");
  } catch (err) {
    assert.ok(err instanceof OversizedFrameError);
    assert.equal(err.bytes, bytes);
    assert.equal(err.limit, limit);
  }
});

// ── isOversizedFrameError ────────────────────────────────────────────────────

test("isOversizedFrameError: true for OversizedFrameError", () => {
  assert.ok(isOversizedFrameError(new OversizedFrameError(100, 50)));
});

test("isOversizedFrameError: false for plain Error", () => {
  assert.equal(isOversizedFrameError(new Error("oops")), false);
});

test("isOversizedFrameError: false for non-error values", () => {
  assert.equal(isOversizedFrameError(null), false);
  assert.equal(isOversizedFrameError("string"), false);
  assert.equal(isOversizedFrameError(42), false);
});
