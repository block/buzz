import assert from "node:assert/strict";
import test from "node:test";

import {
  generateSmoothCornerClipPath,
  generateSmoothCornerPath,
  SMOOTH_CORNER_SMOOTHING,
} from "./smoothCorners.ts";

const radii = {
  topLeft: 16,
  topRight: 16,
  bottomRight: 16,
  bottomLeft: 16,
};

test("smooth corners use the app default smoothing", () => {
  assert.equal(SMOOTH_CORNER_SMOOTHING, 0.6);
});

test("generateSmoothCornerPath keeps the radius while expanding the smoothed shoulder", () => {
  const path = generateSmoothCornerPath(100, 50, radii);

  assert.match(path, /^M 25\.0000 0 L 75\.0000 0/);
  assert.match(path, /a 16\.0000 16\.0000 0 0 1/);
});

test("generateSmoothCornerClipPath emits a CSS path value", () => {
  const clipPath = generateSmoothCornerClipPath(100, 50, radii);

  assert.match(clipPath, /^path\("M /);
  assert.match(clipPath, / Z"\)$/);
});
