import assert from "node:assert/strict";
import { test } from "node:test";

import { relativeTime } from "./projectsViewHelpers.ts";

const DAY_SECONDS = 24 * 60 * 60;

function localSeconds(year, month, day) {
  return Math.floor(new Date(year, month, day, 12).getTime() / 1_000);
}

test("relativeTime switches to an absolute date at seven days", () => {
  const now = localSeconds(2025, 5, 15);

  assert.equal(relativeTime(now - 7 * DAY_SECONDS + 1, now), "6 days ago");

  const createdAt = now - 7 * DAY_SECONDS;
  const expected = new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  assert.equal(relativeTime(createdAt, now), expected);
});

test("relativeTime includes the year only across a year boundary", () => {
  const sameYearNow = localSeconds(2025, 5, 15);
  const sameYearCreatedAt = sameYearNow - 7 * DAY_SECONDS;
  const sameYearExpected = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
  }).format(new Date(sameYearCreatedAt * 1_000));
  assert.equal(relativeTime(sameYearCreatedAt, sameYearNow), sameYearExpected);

  const crossYearNow = localSeconds(2025, 0, 8);
  const crossYearCreatedAt = localSeconds(2024, 11, 31);
  const crossYearExpected = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(crossYearCreatedAt * 1_000));
  assert.equal(
    relativeTime(crossYearCreatedAt, crossYearNow),
    crossYearExpected,
  );
});
