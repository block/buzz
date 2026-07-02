import assert from "node:assert/strict";
import test from "node:test";

import {
  formatDayHeading,
  formatShortMonthDayOrdinal,
  formatThreadSummaryLastReplyTime,
  formatTimeWithoutDayPeriod,
  startOfLocalDaySeconds,
} from "./dateFormatters.ts";

function localUnixSeconds(year, monthIndex, day) {
  return new Date(year, monthIndex, day, 12).getTime() / 1_000;
}

function weekday(date) {
  return new Intl.DateTimeFormat("en-US", { weekday: "long" }).format(date);
}

function month(date) {
  return new Intl.DateTimeFormat("en-US", { month: "long" }).format(date);
}

test("formatShortMonthDayOrdinal formats month before ordinal day", () => {
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 19)),
    "May 19th",
  );
});

test("formatShortMonthDayOrdinal handles ordinal suffixes", () => {
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 1)),
    "May 1st",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 2)),
    "May 2nd",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 3)),
    "May 3rd",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 4)),
    "May 4th",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 11)),
    "May 11th",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 12)),
    "May 12th",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 13)),
    "May 13th",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 21)),
    "May 21st",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 22)),
    "May 22nd",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 23)),
    "May 23rd",
  );
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 31)),
    "May 31st",
  );
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

test("formatThreadSummaryLastReplyTime uses ordinal dates for older replies", () => {
  const now = localUnixSeconds(2026, 5, 15);
  const replyAt = localUnixSeconds(2026, 4, 19);

  assert.equal(formatThreadSummaryLastReplyTime(replyAt, now), "on May 19th");
});

test("formatDayHeading omits the year for current-year dates", () => {
  const now = new Date();
  const date = new Date(now.getFullYear(), (now.getMonth() + 6) % 12, 19, 12);

  assert.equal(
    formatDayHeading(date.getTime() / 1_000),
    `${weekday(date)}, ${month(date)} 19th`,
  );
});

test("formatDayHeading includes the year for other years", () => {
  const year = new Date().getFullYear() - 1;
  const date = new Date(year, 4, 19, 12);

  assert.equal(
    formatDayHeading(date.getTime() / 1_000),
    `${weekday(date)}, May 19th, ${year}`,
  );
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
