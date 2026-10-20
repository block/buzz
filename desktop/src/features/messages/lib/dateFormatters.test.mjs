import assert from "node:assert/strict";
import test from "node:test";

import {
  formatShortMonthDay,
  formatThreadSummaryLastReplyTime,
  formatTimeWithoutDayPeriod,
  startOfLocalDaySeconds,
} from "./dateFormatters.ts";

function localUnixSeconds(year, monthIndex, day) {
  return new Date(year, monthIndex, day, 12).getTime() / 1_000;
}

test("formatShortMonthDay abbreviates the month and omits the ordinal", () => {
  assert.equal(formatShortMonthDay(localUnixSeconds(2026, 4, 19)), "May 19");
  assert.equal(formatShortMonthDay(localUnixSeconds(2026, 4, 1)), "May 1");
});

test("no day carries an ordinal suffix", () => {
  for (const day of [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31]) {
    const label = formatShortMonthDay(localUnixSeconds(2026, 4, day));
    assert.doesNotMatch(label, /\d(?:st|nd|rd|th)\b/, `ordinal in "${label}"`);
  }
});

test("formatTimeWithoutDayPeriod removes AM/PM suffixes", () => {
  assert.equal(formatTimeWithoutDayPeriod("8:00 AM"), "8:00");
  assert.equal(formatTimeWithoutDayPeriod("12:34\u202fPM"), "12:34");
  assert.equal(formatTimeWithoutDayPeriod("16:20"), "16:20");
});

test("formatThreadSummaryLastReplyTime expands relative units", () => {
  const now = localUnixSeconds(2026, 4, 19);

  assert.equal(formatThreadSummaryLastReplyTime(now - 30, now), "just now");
  assert.equal(formatThreadSummaryLastReplyTime(now - 60, now), "1 minute ago");
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 180, now),
    "3 minutes ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 3_600, now),
    "1 hour ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 10_800, now),
    "3 hours ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 86_400, now),
    "1 day ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 345_600, now),
    "4 days ago",
  );
});

test("formatThreadSummaryLastReplyTime dates older replies without an ordinal", () => {
  const now = localUnixSeconds(2026, 5, 15);
  const replyAt = localUnixSeconds(2026, 4, 19);

  assert.equal(formatThreadSummaryLastReplyTime(replyAt, now), "on May 19");
});

test("startOfLocalDaySeconds collapses a day's timestamps to one value", () => {
  const morning = new Date(2026, 5, 14, 8, 30, 15).getTime() / 1_000;
  const evening = new Date(2026, 5, 14, 23, 59, 59).getTime() / 1_000;
  const midnight = new Date(2026, 5, 14, 0, 0, 0).getTime() / 1_000;

  assert.equal(startOfLocalDaySeconds(morning), midnight);
  assert.equal(startOfLocalDaySeconds(evening), midnight);
});

test("startOfLocalDaySeconds separates adjacent calendar days", () => {
  const lateOn14 = new Date(2026, 5, 14, 23, 0, 0).getTime() / 1_000;
  const earlyOn15 = new Date(2026, 5, 15, 1, 0, 0).getTime() / 1_000;

  assert.notEqual(
    startOfLocalDaySeconds(lateOn14),
    startOfLocalDaySeconds(earlyOn15),
  );
});
