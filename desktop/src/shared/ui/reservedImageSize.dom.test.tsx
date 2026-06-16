import assert from "node:assert/strict";
import { test } from "node:test";

// Imported through the DOM lane (esbuild) so we can pull from markdownUtils.ts
// directly even though it imports React — no stale inlined copy. Pure-math
// helper, but it lives in a React module, hence the .dom lane.
import {
  INLINE_IMAGE_MAX_HEIGHT,
  INLINE_IMAGE_MAX_WIDTH,
  reservedImageSize,
} from "./markdownUtils";

test("returns undefined when dim is missing or malformed", () => {
  assert.equal(reservedImageSize(undefined), undefined);
  assert.equal(reservedImageSize(""), undefined);
  assert.equal(reservedImageSize("800"), undefined);
  assert.equal(reservedImageSize("800x"), undefined);
  assert.equal(reservedImageSize("axb"), undefined);
  assert.equal(reservedImageSize("0x600"), undefined);
  assert.equal(reservedImageSize("800x0"), undefined);
});

test("small image within caps is reserved at its natural size", () => {
  // 200x100 fits inside 384x256 → no scaling.
  assert.deepEqual(reservedImageSize("200x100"), { width: 200, height: 100 });
});

test("wide image scales down to the width cap, preserving aspect ratio", () => {
  // 800x400 (2:1). Width cap 384 → scale 0.48 → 384x192.
  assert.deepEqual(reservedImageSize("800x400"), { width: 384, height: 192 });
});

test("tall image scales down to the height cap, preserving aspect ratio", () => {
  // 400x800 (1:2). Height cap 256 → scale 0.32 → 128x256.
  assert.deepEqual(reservedImageSize("400x800"), { width: 128, height: 256 });
});

test("reserved box never exceeds the display caps", () => {
  const r = reservedImageSize("4000x3000");
  assert.ok(r);
  assert.ok(r.width <= INLINE_IMAGE_MAX_WIDTH);
  assert.ok(r.height <= INLINE_IMAGE_MAX_HEIGHT);
  // 4:3 → constrained by height (256) → 341x256.
  assert.deepEqual(r, { width: 341, height: 256 });
});
