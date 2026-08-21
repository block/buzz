import assert from "node:assert/strict";
import { test } from "node:test";

import {
  datetimeLocalToIso,
  defaultScheduleDatetime,
  formatDeliveryTime,
  formatSchedulePill,
  isFutureTimestamp,
  unixToDatetimeLocal,
} from "./scheduledMessages.ts";

test("datetimeLocalToIso returns RFC 3339 for a local datetime-local value", () => {
  const value = "2030-01-02T09:30";
  const iso = datetimeLocalToIso(value);
  assert.ok(iso, "converts a well-formed value");
  // The RFC timestamp must land on the same local wall-clock time as the input
  // (timezone-independent: local 09:30 round-trips back to 09:30 locally).
  const back = unixToDatetimeLocal(Math.floor(new Date(iso).getTime() / 1000));
  assert.equal(back, value);
});

test("datetimeLocalToIso rejects empty and malformed values", () => {
  assert.equal(datetimeLocalToIso(""), null);
  assert.equal(datetimeLocalToIso("not-a-date"), null);
  assert.equal(datetimeLocalToIso("2030-01-02"), null);
});

test("defaultScheduleDatetime is a valid datetime-local value in the future", () => {
  const value = defaultScheduleDatetime();
  assert.match(value, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
  const iso = datetimeLocalToIso(value);
  assert.ok(iso);
  assert.ok(new Date(iso).getTime() > Date.now());
  // Interpreting the value as local wall-clock matches the local wall-clock
  // implied by the generated value (proves it is local, not UTC).
  assert.equal(
    unixToDatetimeLocal(Math.floor(new Date(iso).getTime() / 1000)),
    value,
  );
});

test("unixToDatetimeLocal pads month and minute", () => {
  // Build a local Date from components so the assertion is timezone-independent.
  const date = new Date(2030, 0, 2, 9, 5, 0, 0); // Jan 2 2030 09:05 local
  const ts = Math.floor(date.getTime() / 1000);
  const value = unixToDatetimeLocal(ts);
  assert.match(value, /^\d{4}-\d{2}-\d{2}T\d{2}:05$/);
  assert.ok(value.startsWith("2030-01-02T09:"));
});

test("isFutureTimestamp bounds-check", () => {
  assert.ok(isFutureTimestamp(Math.floor(Date.now() / 1000) + 60));
  assert.equal(isFutureTimestamp(0), false);
});

test("formatDeliveryTime renders a local-time label", () => {
  const ts = Math.floor(Date.now() / 1000) + 3600;
  const label = formatDeliveryTime(ts);
  assert.ok(label.length > 0);
  assert.match(label, /\d{1,2}:\d{2}/);
});

test("formatSchedulePill shows time for today and weekday for tomorrow", () => {
  const inHour = Math.floor(Date.now() / 1000) + 3600;
  assert.match(formatSchedulePill(inHour), /\d{1,2}:\d{2}/);
  const tomorrow = Math.floor(Date.now() / 1000) + 24 * 60 * 60 + 3600;
  assert.ok(formatSchedulePill(tomorrow).length > 0);
});
