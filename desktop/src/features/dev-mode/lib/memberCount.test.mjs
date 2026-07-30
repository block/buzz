import assert from "node:assert/strict";
import test from "node:test";

import { formatMemberCount, memberCountLabel } from "./memberCount.ts";

test("formatMemberCount_belowCapIsExact", () => {
  assert.equal(formatMemberCount(0), "0");
  assert.equal(formatMemberCount(1), "1");
  assert.equal(formatMemberCount(999), "999");
});

test("formatMemberCount_atCapIsLowerBound", () => {
  assert.equal(formatMemberCount(1000), "1000+");
});

test("formatMemberCount_aboveCapStillSaturates", () => {
  // Defensive: a future relay may lift the cap; never show a bogus exact
  // number above it until the client is updated deliberately.
  assert.equal(formatMemberCount(1234), "1000+");
});

test("memberCountLabel_pluralizes", () => {
  assert.equal(memberCountLabel(1), "1 member");
  assert.equal(memberCountLabel(2), "2 members");
  assert.equal(memberCountLabel(1000), "1000+ members");
});
