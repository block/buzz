import assert from "node:assert/strict";
import test from "node:test";

import {
  formatAnimatedCount,
  getAnimatedCountSlots,
  normalizeAnimatedCount,
} from "./animatedCountParts.ts";

test("normalizeAnimatedCount clamps to displayable whole counts", () => {
  assert.equal(normalizeAnimatedCount(4.9), 4);
  assert.equal(normalizeAnimatedCount(-1), 0);
  assert.equal(normalizeAnimatedCount(Number.NaN), 0);
});

test("formatAnimatedCount uses stable grouped ASCII digits", () => {
  assert.equal(formatAnimatedCount(1234), "1,234");
});

test("getAnimatedCountSlots right-aligns digits for rollover", () => {
  assert.deepEqual(getAnimatedCountSlots("9", "10"), [
    { current: "1", isDigit: true, place: 1, previous: "" },
    { current: "0", isDigit: true, place: 0, previous: "9" },
  ]);
});

test("getAnimatedCountSlots shrinks to current width after settling", () => {
  assert.equal(getAnimatedCountSlots("1,000", "999").length, 5);
  assert.deepEqual(getAnimatedCountSlots("999", "999"), [
    { current: "9", isDigit: true, place: 2, previous: "9" },
    { current: "9", isDigit: true, place: 1, previous: "9" },
    { current: "9", isDigit: true, place: 0, previous: "9" },
  ]);
});
