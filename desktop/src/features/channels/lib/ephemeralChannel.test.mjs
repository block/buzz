import assert from "node:assert/strict";
import test from "node:test";

import { formatTtlDuration, parseTtlDuration } from "./ephemeralChannel.ts";

test("parseTtlDuration parses single units", () => {
  assert.equal(parseTtlDuration("45s"), 45);
  assert.equal(parseTtlDuration("30m"), 30 * 60);
  assert.equal(parseTtlDuration("12h"), 12 * 60 * 60);
  assert.equal(parseTtlDuration("1d"), 24 * 60 * 60);
});

test("parseTtlDuration parses combined units and tolerates whitespace/case", () => {
  assert.equal(parseTtlDuration("1d12h"), 36 * 60 * 60);
  assert.equal(parseTtlDuration(" 1D 12H "), 36 * 60 * 60);
  assert.equal(parseTtlDuration("1h30m"), 90 * 60);
});

test("parseTtlDuration rejects malformed input", () => {
  assert.equal(parseTtlDuration(""), null);
  assert.equal(parseTtlDuration("   "), null);
  assert.equal(parseTtlDuration("100"), null); // no unit
  assert.equal(parseTtlDuration("1x"), null); // bad unit
  assert.equal(parseTtlDuration("1d!"), null); // trailing junk
  assert.equal(parseTtlDuration("abc"), null);
  assert.equal(parseTtlDuration("0m"), null); // zero total
  assert.equal(parseTtlDuration("1d1d"), null); // duplicate unit
});

test("formatTtlDuration is the inverse for common values", () => {
  assert.equal(formatTtlDuration(45), "45s");
  assert.equal(formatTtlDuration(30 * 60), "30m");
  assert.equal(formatTtlDuration(12 * 60 * 60), "12h");
  assert.equal(formatTtlDuration(24 * 60 * 60), "1d");
  assert.equal(formatTtlDuration(36 * 60 * 60), "1d12h");
  assert.equal(formatTtlDuration(90 * 60), "1h30m");
});

test("formatTtlDuration handles non-positive input", () => {
  assert.equal(formatTtlDuration(0), "");
  assert.equal(formatTtlDuration(-5), "");
});

test("parse/format round-trip", () => {
  for (const s of ["30m", "12h", "1d", "1d12h", "1h30m", "45s"]) {
    assert.equal(formatTtlDuration(parseTtlDuration(s)), s);
  }
});
