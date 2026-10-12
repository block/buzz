import assert from "node:assert/strict";
import test from "node:test";
import { IDENTITY, MAX_SCALE, baseSize, clampView, maxPan, panBy, toggleZoom, wheelFactor, zoomAt } from "./zoomPan.ts";

const stage = { w: 400, h: 800 };
const base = baseSize(stage, 0.5); // 400x800: exactly fits

test("base size fills the stage height; wide images overflow horizontally and may pan at 1x", () => {
  assert.deepEqual(baseSize(stage, 2), { w: 1600, h: 800 });
  assert.deepEqual(maxPan(base, stage, 1), { x: 0, y: 0 });
  assert.deepEqual(maxPan(baseSize(stage, 2), stage, 1), { x: 600, y: 0 });
  assert.deepEqual(maxPan(base, stage, 2), { x: 200, y: 400 });
});

test("zoom is clamped to 1..8 and pan is clamped so the image always covers the stage", () => {
  assert.equal(zoomAt(IDENTITY, 100, { x: 0, y: 0 }, base, stage).scale, MAX_SCALE);
  assert.deepEqual(zoomAt(IDENTITY, 0.1, { x: 0, y: 0 }, base, stage), IDENTITY);
  const z = zoomAt(IDENTITY, 2, { x: 0, y: 0 }, base, stage);
  assert.deepEqual(panBy(z, 9999, -9999, base, stage), { scale: 2, x: 200, y: -400 });
  assert.deepEqual(clampView({ scale: 1, x: 50, y: 50 }, base, stage), IDENTITY);
});

test("zoomAt keeps the point under the cursor fixed", () => {
  const at = { x: 100, y: 200 };
  const z = zoomAt(IDENTITY, 2, at, base, stage);
  // image point under `at` before: (100,200); after: (at - t)/scale must equal it
  assert.equal((at.x - z.x) / z.scale, 100);
  assert.equal((at.y - z.y) / z.scale, 200);
});

test("double tap toggles between 1x and zoomed; wheel factor is smooth and signed", () => {
  const z = toggleZoom(IDENTITY, { x: 50, y: 50 }, base, stage);
  assert.ok(z.scale > 2);
  assert.deepEqual(toggleZoom(z, { x: 0, y: 0 }, base, stage), IDENTITY);
  assert.ok(wheelFactor(-100) > 1 && wheelFactor(100) < 1 && wheelFactor(0) === 1);
});
