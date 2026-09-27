import assert from "node:assert/strict";
import test from "node:test";

import {
  applyAgentPlaygroundViewport,
  clearPlaygroundViewport,
  getPlaygroundViewport,
  normalizeAgentViewportRequest,
  playgroundViewportCaption,
  resetPlaygroundViewports,
  setPlaygroundViewport,
} from "./playgroundViewport.ts";

test("caption: Desktop without dims", () => {
  assert.equal(
    playgroundViewportCaption({ mode: "desktop", width: 0, height: 0 }),
    "Desktop",
  );
});

test("caption: Desktop · W×H", () => {
  assert.equal(
    playgroundViewportCaption({ mode: "desktop", width: 1280, height: 800 }),
    "Desktop · 1280×800",
  );
});

test("caption: Responsive · W×H", () => {
  assert.equal(
    playgroundViewportCaption({
      mode: "responsive",
      width: 390,
      height: 844,
    }),
    "Responsive · 390×844",
  );
});

test("caption: Mobile dims; scale only when not 100%", () => {
  assert.equal(
    playgroundViewportCaption({
      mode: "mobile",
      width: 393,
      height: 852,
      scalePercent: 100,
    }),
    "Mobile · 393×852",
  );
  assert.equal(
    playgroundViewportCaption({
      mode: "mobile",
      width: 590,
      height: 1278,
      scalePercent: 150,
    }),
    "Mobile · 590×1278 · 150%",
  );
});

test("set/get/clear viewport store", () => {
  resetPlaygroundViewports();
  assert.equal(getPlaygroundViewport("a").mode, "desktop");
  setPlaygroundViewport("a", {
    mode: "mobile",
    width: 393,
    height: 852,
    scalePercent: 125,
    deviceId: "iphone-16",
    orientation: "portrait",
  });
  assert.deepEqual(getPlaygroundViewport("a"), {
    mode: "mobile",
    width: 393,
    height: 852,
    scalePercent: 125,
    deviceId: "iphone-16",
    orientation: "portrait",
  });
  clearPlaygroundViewport("a");
  assert.equal(getPlaygroundViewport("a").mode, "desktop");
  resetPlaygroundViewports();
});

test("normalizeAgentViewportRequest: desktop", () => {
  const r = normalizeAgentViewportRequest({ mode: "desktop" });
  assert.equal(r.ok, true);
  if (!r.ok) return;
  assert.equal(r.snapshot.mode, "desktop");
});

test("normalizeAgentViewportRequest: responsive clamps min 320", () => {
  const r = normalizeAgentViewportRequest({
    mode: "responsive",
    width: 100,
    height: 200,
  });
  assert.equal(r.ok, true);
  if (!r.ok) return;
  assert.deepEqual(r.snapshot, {
    mode: "responsive",
    width: 320,
    height: 320,
  });
});

test("normalizeAgentViewportRequest: mobile device + landscape + scale", () => {
  const r = normalizeAgentViewportRequest({
    mode: "mobile",
    deviceId: "iphone-16",
    orientation: "landscape",
    scalePercent: 75,
  });
  assert.equal(r.ok, true);
  if (!r.ok) return;
  assert.equal(r.snapshot.mode, "mobile");
  assert.equal(r.snapshot.deviceId, "iphone-16");
  assert.equal(r.snapshot.orientation, "landscape");
  assert.equal(r.snapshot.scalePercent, 75);
  // 852×393 @ 75%
  assert.equal(r.snapshot.width, Math.round(852 * 0.75));
  assert.equal(r.snapshot.height, Math.round(393 * 0.75));
});

test("normalizeAgentViewportRequest: rejects bad mode/device", () => {
  assert.equal(normalizeAgentViewportRequest({ mode: "tablet" }).ok, false);
  assert.equal(
    normalizeAgentViewportRequest({
      mode: "mobile",
      deviceId: "nokia-3310",
    }).ok,
    false,
  );
  assert.equal(
    normalizeAgentViewportRequest({
      mode: "mobile",
      orientation: "upside-down",
    }).ok,
    false,
  );
});

test("applyAgentPlaygroundViewport writes store", () => {
  resetPlaygroundViewports();
  const r = applyAgentPlaygroundViewport("sid-1", {
    mode: "responsive",
    width: 414,
    height: 896,
  });
  assert.equal(r.ok, true);
  assert.deepEqual(getPlaygroundViewport("sid-1"), {
    mode: "responsive",
    width: 414,
    height: 896,
  });
  resetPlaygroundViewports();
});
