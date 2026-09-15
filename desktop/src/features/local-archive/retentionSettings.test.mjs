import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_RETENTION_DAYS,
  MIN_RETENTION_DAYS,
  deriveSizeReadout,
  formatBytes,
  parseRetentionDays,
} from "./retentionSettings.ts";

// ── parseRetentionDays ────────────────────────────────────────────────────────

test("parseRetentionDays_validInteger_ok", () => {
  assert.deepEqual(parseRetentionDays("30"), { ok: true, days: 30 });
});

test("parseRetentionDays_trimsWhitespace", () => {
  assert.deepEqual(parseRetentionDays("  7  "), { ok: true, days: 7 });
});

test("parseRetentionDays_min_ok", () => {
  assert.deepEqual(parseRetentionDays(String(MIN_RETENTION_DAYS)), {
    ok: true,
    days: MIN_RETENTION_DAYS,
  });
});

test("parseRetentionDays_max_ok", () => {
  assert.deepEqual(parseRetentionDays(String(MAX_RETENTION_DAYS)), {
    ok: true,
    days: MAX_RETENTION_DAYS,
  });
});

test("parseRetentionDays_empty_error", () => {
  const r = parseRetentionDays("   ");
  assert.equal(r.ok, false);
});

test("parseRetentionDays_zero_belowMin", () => {
  const r = parseRetentionDays("0");
  assert.equal(r.ok, false);
  assert.match(r.error, /at least 1/);
});

test("parseRetentionDays_negative_rejected", () => {
  // The leading "-" fails the digits-only shape before the range check.
  const r = parseRetentionDays("-5");
  assert.equal(r.ok, false);
  assert.match(r.error, /whole number/);
});

test("parseRetentionDays_fractional_rejected", () => {
  const r = parseRetentionDays("3.5");
  assert.equal(r.ok, false);
  assert.match(r.error, /whole number/);
});

test("parseRetentionDays_nonNumeric_rejected", () => {
  const r = parseRetentionDays("abc");
  assert.equal(r.ok, false);
});

test("parseRetentionDays_aboveMax_rejected", () => {
  const r = parseRetentionDays(String(MAX_RETENTION_DAYS + 1));
  assert.equal(r.ok, false);
  assert.match(r.error, /at most/);
});

// ── formatBytes ───────────────────────────────────────────────────────────────

test("formatBytes_zeroAndNegative_clampToZero", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(-100), "0 B");
  assert.equal(formatBytes(Number.NaN), "0 B");
});

test("formatBytes_bytes", () => {
  assert.equal(formatBytes(820), "820 B");
});

test("formatBytes_kilobytes_oneDecimalUnderTen", () => {
  assert.equal(formatBytes(1024 * 3 + 410), "3.4 KB");
});

test("formatBytes_gigabytes", () => {
  assert.equal(formatBytes(7.4 * 1024 ** 3), "7.4 GB");
});

test("formatBytes_roundsAtTenOrAbove", () => {
  // 12.4 GB rounds to whole number once ≥ 10 in its unit.
  assert.equal(formatBytes(12.4 * 1024 ** 3), "12 GB");
});

// ── deriveSizeReadout ─────────────────────────────────────────────────────────

test("deriveSizeReadout_physicalIsMainPlusWal", () => {
  const r = deriveSizeReadout({
    mainFileBytes: 1024 ** 3,
    walFileBytes: 512 * 1024 ** 2,
    pageSize: 4096,
    pageCount: 1000,
    freelistCount: 0,
  });
  assert.equal(r.physicalBytes, 1024 ** 3 + 512 * 1024 ** 2);
  assert.equal(r.physicalLabel, "1.5 GB");
  assert.equal(r.reclaimableBytes, 0);
  assert.equal(r.reclaimableLabel, "0 B");
});

test("deriveSizeReadout_reclaimableIsFreelistTimesPageSize", () => {
  const r = deriveSizeReadout({
    mainFileBytes: 0,
    walFileBytes: 0,
    pageSize: 4096,
    freelistCount: 262_144, // 256K pages × 4KB = 1 GB
    pageCount: 300_000,
  });
  assert.equal(r.reclaimableBytes, 1024 ** 3);
  assert.equal(r.reclaimableLabel, "1.0 GB");
});
