import assert from "node:assert/strict";
import { test } from "node:test";

import { applyReadBaseline, computeReadBaseline } from "./readStateBaseline.ts";

test("baseline is the latest synced marker, else first-login time", () => {
  assert.equal(computeReadBaseline([100, 500, 300], 1_000), 500);
  assert.equal(computeReadBaseline([], 1_000), 1_000);
  // future-skewed markers are ignored
  assert.equal(computeReadBaseline([400, 99_999], 1_000), 400);
});

test("markers are floored at the baseline", () => {
  assert.equal(applyReadBaseline(null, 500), 500);
  assert.equal(applyReadBaseline(200, 500), 500);
  assert.equal(applyReadBaseline(800, 500), 800);
  assert.equal(applyReadBaseline(null, null), null);
});
