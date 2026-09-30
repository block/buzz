import assert from "node:assert/strict";
import test from "node:test";

import { buildLocalMidnightBucketBoundaries } from "./useAgentUsageSeries.ts";

test("buildLocalMidnightBucketBoundaries returns dayCount+1 entries", () => {
  const boundaries = buildLocalMidnightBucketBoundaries(7, Date.UTC(2026, 8, 29, 18, 0, 0));
  assert.equal(boundaries.length, 8);
  for (let i = 0; i < boundaries.length - 1; i += 1) {
    assert.ok(boundaries[i] < boundaries[i + 1], `boundary ${i} must increase`);
  }
});

test("buildLocalMidnightBucketBoundaries rejects out-of-range dayCount", () => {
  assert.throws(() => buildLocalMidnightBucketBoundaries(0));
  assert.throws(() => buildLocalMidnightBucketBoundaries(31));
});
