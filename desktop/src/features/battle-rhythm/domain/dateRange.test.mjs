import assert from "node:assert/strict";
import test from "node:test";

import {
  getMonthCells,
  getWeekRange,
  getYearRange,
  localDateTimeToRfc3339,
  overlapsRange,
} from "./dateRange.ts";

test("year ranges support 12 and 24 months", () => {
  assert.deepEqual(getYearRange("2026-07-28", "Australia/Sydney", 12), {
    start: "2026-01-01T00:00:00+11:00",
    end: "2027-01-01T00:00:00+11:00",
  });
  assert.deepEqual(getYearRange("2026-07-28", "Australia/Sydney", 24), {
    start: "2026-01-01T00:00:00+11:00",
    end: "2028-01-01T00:00:00+11:00",
  });
});

test("weeks begin on Monday across Sydney daylight-saving changes", () => {
  assert.deepEqual(getWeekRange("2026-10-04", "Australia/Sydney"), {
    start: "2026-09-28T00:00:00+10:00",
    end: "2026-10-05T00:00:00+11:00",
  });
});

test("month cells include leading and trailing calendar days", () => {
  const cells = getMonthCells("2026-02-15", "Australia/Sydney");
  assert.equal(cells.length, 35);
  assert.equal(cells[0], "2026-01-26");
  assert.equal(cells.at(-1), "2026-03-01");
});

test("range overlap includes timed and all-day events touching the window", () => {
  const range = {
    start: "2026-10-04T00:00:00+10:00",
    end: "2026-10-05T00:00:00+11:00",
  };
  assert.equal(
    overlapsRange(
      "2026-10-04T10:00:00+11:00",
      "2026-10-04T11:00:00+11:00",
      range,
    ),
    true,
  );
  assert.equal(
    overlapsRange(
      "2026-10-03T23:00:00+10:00",
      "2026-10-04T01:00:00+10:00",
      range,
    ),
    true,
  );
  assert.equal(
    overlapsRange(
      "2026-10-05T00:00:00+11:00",
      "2026-10-05T01:00:00+11:00",
      range,
    ),
    false,
  );
});

test("local calendar input converts with the Sydney offset across daylight saving", () => {
  assert.equal(
    localDateTimeToRfc3339("2026-07-29T08:00", "Australia/Sydney"),
    "2026-07-29T08:00:00+10:00",
  );
  assert.equal(
    localDateTimeToRfc3339("2026-10-05T08:00", "Australia/Sydney"),
    "2026-10-05T08:00:00+11:00",
  );
});
