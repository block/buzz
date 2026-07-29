import assert from "node:assert/strict";
import test from "node:test";

import {
  calendarHeading,
  formatShipTime,
  monthGrid,
  weekDayHeading,
} from "./calendarPresentation.ts";

test("formats all four calendar horizons and 24-hour Ship Time", () => {
  assert.equal(
    calendarHeading("Day", "2026-07-29", "Australia/Sydney"),
    "Wednesday, 29 July 2026",
  );
  assert.equal(
    calendarHeading("Week", "2026-07-29", "Australia/Sydney"),
    "27 July – 2 August 2026",
  );
  assert.equal(
    calendarHeading("Month", "2026-07-29", "Australia/Sydney"),
    "July 2026",
  );
  assert.equal(
    calendarHeading("Year", "2026-07-29", "Australia/Sydney"),
    "2026",
  );
  assert.equal(
    formatShipTime("2026-07-29T07:05:00+10:00", "Australia/Sydney"),
    "07:05",
  );
});

test("week headings include weekday, date, and month", () => {
  assert.equal(weekDayHeading("2026-07-27", "Australia/Sydney"), "MON 27 JUL");
  assert.equal(weekDayHeading("2026-08-02", "Australia/Sydney"), "SUN 2 AUG");
});

test("year presentation is a conventional twelve-month grid", () => {
  const months = monthGrid("2026-07-29", "Australia/Sydney");
  assert.equal(months.length, 12);
  assert.equal(months[0].label, "January");
  assert.equal(months[11].label, "December");
  assert.equal(months[0].cells.length, 35);
  assert.equal(months[1].cells.length, 35);
  assert.equal(months[6].cells[0], "2026-06-29");
});
