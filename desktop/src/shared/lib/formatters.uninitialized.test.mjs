/**
 * English before i18n has booted (spec NFR-003: the fallback always answers).
 *
 * `main.tsx` initializes i18n before the first render, but a formatter called
 * from module-init order that outruns it — or from a test process that never
 * boots i18n at all — must still print English, not `common.date.today` and
 * not a Chinese word. Nothing here imports `initializeI18n`; that absence is
 * the subject under test, so this file stays separate from the pinned-English
 * suites.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { formatDayGroupLabel, formatItemTimestamp } from "./datetime.ts";
import { appIntlLocale, dateWords, joinDayAndTime } from "./formatters.ts";

function at(year, monthIndex, day, hour = 12, minute = 0) {
  return new Date(year, monthIndex, day, hour, minute).getTime() / 1_000;
}

const NOW = at(2026, 6, 30, 14, 30);

test("no bare keys reach the screen before i18n initializes", () => {
  assert.deepEqual(dateWords(), { today: "Today", yesterday: "Yesterday" });
  assert.equal(joinDayAndTime("Yesterday", "2:34 PM"), "Yesterday at 2:34 PM");
  assert.equal(formatDayGroupLabel(at(2026, 6, 30), NOW), "Today");
  assert.equal(formatDayGroupLabel(at(2026, 6, 29), NOW), "Yesterday");
  assert.equal(
    formatItemTimestamp(at(2026, 6, 29, 14, 34), {
      withTime: true,
      nowSeconds: NOW,
    }),
    "Yesterday at 2:34 PM",
  );
});

test("the pre-init formatter locale is the en-US the module always used", () => {
  assert.equal(appIntlLocale(), "en-US");
  assert.equal(
    formatItemTimestamp(at(2026, 6, 30, 9, 5), { nowSeconds: NOW }),
    "9:05 AM",
  );
});
