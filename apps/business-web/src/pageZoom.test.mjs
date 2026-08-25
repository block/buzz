import assert from "node:assert/strict";
import test from "node:test";
import { stepPageZoom } from "./pageZoom.ts";

test("steps page zoom through bounded business UI presets", () => {
  assert.equal(stepPageZoom(1, 1), 1.1);
  assert.equal(stepPageZoom(1, -1), 0.9);
  assert.equal(stepPageZoom(1.5, 1), 1.5);
  assert.equal(stepPageZoom(0.8, -1), 0.8);
});

test("moves an off-preset zoom from its nearest step", () => {
  assert.equal(stepPageZoom(1.04, 1), 1.1);
  assert.equal(stepPageZoom(1.04, -1), 0.9);
});
