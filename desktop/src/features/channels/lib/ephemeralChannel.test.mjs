import assert from "node:assert/strict";
import test from "node:test";

import { initializeI18n } from "@/i18n/index.ts";
import {
  formatTtlDuration,
  getEphemeralChannelDisplay,
  parseTtlDuration,
} from "./ephemeralChannel.ts";

// The badge and tooltip builders resolve their text through `i18n.t`, which
// returns `undefined` until the singleton boots (`main.tsx` does that in the
// app). English is pinned explicitly before init: node's own
// `navigator.languages` reports the host system locale — which may be zh-CN —
// and every assertion below is the English contract.
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { languages: ["en-US", "en"], userAgent: "buzz-unit-test" },
});
initializeI18n();

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

const NOW = Date.parse("2026-03-05T09:00:00Z");
const deadlineAt = (offsetSeconds) =>
  new Date(NOW + offsetSeconds * 1000).toISOString();
const display = (ttlSeconds, ttlDeadline) =>
  getEphemeralChannelDisplay({ ttlSeconds, ttlDeadline }, NOW);

test("getEphemeralChannelDisplay badges a TTL-only channel", () => {
  assert.deepEqual(display(7 * 24 * 60 * 60, null), {
    detailLabel: "7d TTL",
    tooltipLabel: "Ephemeral channel. Cleans up after 7 days of inactivity.",
  });
  assert.deepEqual(display(null, null), null);
});

test("getEphemeralChannelDisplay pluralizes the inactivity duration", () => {
  assert.equal(
    display(60, null).tooltipLabel,
    "Ephemeral channel. Cleans up after 1 minute of inactivity.",
  );
  assert.equal(
    display(45, null).tooltipLabel,
    "Ephemeral channel. Cleans up after 45 seconds of inactivity.",
  );
  assert.equal(display(60, null).detailLabel, "1m TTL");
  assert.equal(display(45, null).detailLabel, "45s TTL");
  assert.equal(
    display(7200, null).tooltipLabel,
    "Ephemeral channel. Cleans up after 2 hours of inactivity.",
  );
});

test("getEphemeralChannelDisplay counts down to the deadline", () => {
  assert.equal(display(null, deadlineAt(30)).detailLabel, "1m left");
  assert.equal(display(null, deadlineAt(30 * 60)).detailLabel, "30m left");
  assert.equal(display(null, deadlineAt(2 * 60 * 60)).detailLabel, "2h left");
  assert.equal(
    display(null, deadlineAt(5 * 24 * 60 * 60)).detailLabel,
    "5d left",
  );
  // The absolute timestamp the tooltip appends is host-time-zone dependent, so
  // only its shape is asserted here.
  assert.match(
    display(null, deadlineAt(30 * 60)).tooltipLabel,
    /^Ephemeral channel\. Cleans up in 30 minutes\. Scheduled for .+\.$/,
  );
});

test("getEphemeralChannelDisplay flags a passed deadline as due", () => {
  assert.deepEqual(display(null, deadlineAt(-60 * 60)), {
    detailLabel: "Cleanup due",
    tooltipLabel: "Ephemeral channel. Cleanup is due now.",
  });
});
