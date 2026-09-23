import assert from "node:assert/strict";
import test from "node:test";

import {
  afterPlaygroundLayout,
  playgroundDeviceBezel,
  playgroundDeviceBezelOuterSize,
  playgroundDeviceFrameSize,
  playgroundDeviceNubGutter,
  playgroundStageBoundsSyncTargets,
  playgroundStageChromeElement,
  playgroundStageMeasureElement,
  readPlaygroundStageBounds,
  rotateBezelEdge,
  scalePlaygroundDeviceBezel,
} from "./deviceBezel.ts";
import { PLAYGROUND_DEVICES, playgroundDeviceViewport } from "./devices.ts";

function device(id) {
  return PLAYGROUND_DEVICES.find((item) => item.id === id);
}

test("phone bezels are thicker than the CSS viewport and use large radii", () => {
  const iphone = device("iphone-16");
  const viewport = playgroundDeviceViewport(iphone, "portrait");
  const bezel = playgroundDeviceBezel(iphone, "portrait");
  const outer = playgroundDeviceBezelOuterSize(viewport, bezel);
  assert.ok(outer.width > viewport.width);
  assert.ok(outer.height > viewport.height);
  assert.equal(
    outer.width - viewport.width,
    bezel.padding.left + bezel.padding.right,
  );
  assert.equal(
    outer.height - viewport.height,
    bezel.padding.top + bezel.padding.bottom,
  );
  assert.ok(bezel.outerRadius >= 48);
  assert.ok(bezel.innerRadius >= 32);
  assert.equal(bezel.chrome, "iphone-island");
  assert.ok(bezel.island);
  assert.ok(bezel.homeIndicator);
  assert.doesNotMatch(bezel.bezelColor, /backdrop|blur|\//);
});

test("iPhone SE uses a home-button chin instead of an island", () => {
  const se = playgroundDeviceBezel(device("iphone-se"), "portrait");
  assert.equal(se.chrome, "iphone-home-button");
  assert.equal(se.island, undefined);
  assert.ok(se.homeButton);
  assert.ok(se.padding.bottom > se.padding.top);
});

test("Pixel chrome is a punch, not an Apple island", () => {
  const pixel = playgroundDeviceBezel(device("pixel-8"), "portrait");
  assert.equal(pixel.family, "pixel");
  assert.equal(pixel.chrome, "pixel-punch");
  assert.ok(pixel.punch);
  assert.equal(pixel.island, undefined);
  assert.ok(pixel.outerRadius >= 48);
});

test("iPad chrome is a thin tablet bezel without phone island", () => {
  const ipad = playgroundDeviceBezel(device("ipad-pro-11"), "portrait");
  assert.equal(ipad.family, "ipad");
  assert.equal(ipad.chrome, "ipad");
  assert.equal(ipad.island, undefined);
  assert.equal(ipad.punch, undefined);
  assert.ok(ipad.cameraDot);
  assert.ok(ipad.padding.top <= 16);
  assert.ok(ipad.outerRadius >= 32);
  assert.equal(playgroundDeviceNubGutter(ipad), 0);
});

test("landscape rotates padding and nubs onto the long edge", () => {
  const iphone = device("iphone-16");
  const portrait = playgroundDeviceBezel(iphone, "portrait");
  const landscape = playgroundDeviceBezel(iphone, "landscape");
  assert.equal(landscape.padding.left, portrait.padding.top);
  assert.equal(landscape.padding.right, portrait.padding.bottom);
  assert.equal(rotateBezelEdge("top"), "left");
  assert.equal(rotateBezelEdge("right"), "top");
  assert.ok(
    landscape.nubs.every((nub) => nub.edge === "top" || nub.edge === "bottom"),
  );
  const viewport = playgroundDeviceViewport(iphone, "landscape");
  const outer = playgroundDeviceFrameSize(iphone, "landscape");
  assert.ok(outer.width > viewport.width);
  assert.ok(outer.height > viewport.height);
});

test("native stage bounds come from the live inner screen rect, not CSS viewport", () => {
  const screen = {
    getBoundingClientRect: () => ({
      x: 48,
      y: 80,
      left: 48,
      top: 80,
      width: 393,
      height: 852,
      bottom: 932,
    }),
    closest(selector) {
      return selector.includes("playground-device-screen") ? this : null;
    },
  };
  const frame = {
    getBoundingClientRect: () => ({
      x: 20,
      y: 20,
      left: 20,
      top: 20,
      width: 425,
      height: 920,
      bottom: 940,
    }),
    closest() {
      return null;
    },
  };
  const viewport = { width: 393, height: 852 };
  // Live screen hole wins — do not substitute a mismatched CSS viewport size.
  assert.deepEqual(readPlaygroundStageBounds(screen, viewport), {
    x: 48,
    y: 80,
    width: 393,
    height: 852,
  });
  assert.notDeepEqual(
    readPlaygroundStageBounds(screen, viewport),
    readPlaygroundStageBounds(frame),
  );
});

test("nested webview host measures the museum screen hole", () => {
  const screen = {
    id: "screen",
    getBoundingClientRect: () => ({
      x: 120,
      y: 90,
      left: 120,
      top: 90,
      width: 393,
      height: 852,
      bottom: 942,
    }),
  };
  const host = {
    id: "host",
    getBoundingClientRect: () => ({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      width: 10,
      height: 10,
      bottom: 10,
    }),
    closest(selector) {
      return selector.includes("playground-device-screen") ? screen : null;
    },
  };
  assert.equal(playgroundStageMeasureElement(host), screen);
  assert.deepEqual(readPlaygroundStageBounds(host), {
    x: 120,
    y: 90,
    width: 393,
    height: 852,
  });
});

test("viewport size is only a fallback while the screen hole is empty", () => {
  const empty = {
    getBoundingClientRect: () => ({
      x: 40,
      y: 60,
      left: 40,
      top: 60,
      width: 0,
      height: 0,
      bottom: 60,
    }),
    closest() {
      return null;
    },
  };
  assert.deepEqual(
    readPlaygroundStageBounds(empty, { width: 393, height: 852 }),
    { x: 40, y: 60, width: 393, height: 852 },
  );
});

test("native stage bounds never overlap playground chrome", () => {
  const host = {
    getBoundingClientRect: () => ({
      x: 0,
      y: 40,
      left: 0,
      top: 40,
      width: 400,
      height: 360,
      bottom: 400,
    }),
    closest() {
      return null;
    },
  };
  const overlapping = { getBoundingClientRect: () => ({ bottom: 72 }) };
  assert.deepEqual(readPlaygroundStageBounds(host, undefined, overlapping), {
    x: 0,
    y: 72,
    width: 400,
    height: 328,
  });
  // Live laid-out size wins over published viewport once the hole is non-zero.
  assert.deepEqual(
    readPlaygroundStageBounds(host, { width: 393, height: 852 }, overlapping),
    { x: 0, y: 72, width: 400, height: 328 },
  );
  const flush = { getBoundingClientRect: () => ({ bottom: 40 }) };
  assert.deepEqual(readPlaygroundStageBounds(host, undefined, flush), {
    x: 0,
    y: 40,
    width: 400,
    height: 360,
  });
});

test("scalePlaygroundDeviceBezel multiplies linear chrome for museum scale", () => {
  const portrait = playgroundDeviceBezel(device("iphone-16"), "portrait");
  const scaled = scalePlaygroundDeviceBezel(portrait, 0.5);
  assert.equal(scaled.padding.top, Math.round(portrait.padding.top * 0.5));
  assert.equal(scaled.outerRadius, Math.round(portrait.outerRadius * 0.5));
  assert.equal(scaled.island?.width, Math.round(portrait.island.width * 0.5));
  assert.equal(scalePlaygroundDeviceBezel(portrait, 1), portrait);
});

test("stage sync targets include museum backdrop and device frame", () => {
  const overlay = { id: "overlay" };
  const chrome = { id: "chrome" };
  const stage = { id: "stage" };
  const backdrop = { id: "backdrop" };
  const frame = { id: "frame" };
  const screen = { id: "screen" };
  const host = {
    id: "host",
    closest(selector) {
      if (selector.includes("playground-overlay")) return overlay;
      if (selector.includes("playground-mobile-backdrop")) return backdrop;
      if (selector.includes("playground-mobile-stage")) return stage;
      if (selector.includes("playground-device-frame")) return frame;
      if (selector.includes("playground-device-screen")) return screen;
      return null;
    },
  };
  overlay.querySelector = (selector) =>
    selector.includes("playground-chrome") ? chrome : null;

  const targets = playgroundStageBoundsSyncTargets(host);
  assert.deepEqual(
    targets.observe.map((el) => el.id),
    ["host", "chrome", "backdrop", "stage", "frame", "screen", "overlay"],
  );
  assert.deepEqual(
    targets.scroll.map((el) => el.id),
    ["backdrop"],
  );
  assert.equal(playgroundStageChromeElement(host), chrome);
});

test("afterPlaygroundLayout waits for nested animation frames", () => {
  const calls = [];
  const originalRaf = globalThis.requestAnimationFrame;
  const originalCancel = globalThis.cancelAnimationFrame;
  const queue = [];
  globalThis.requestAnimationFrame = (cb) => {
    queue.push(cb);
    return queue.length;
  };
  globalThis.cancelAnimationFrame = (id) => {
    queue[id - 1] = null;
  };
  try {
    const cancel = afterPlaygroundLayout(() => calls.push("run"));
    assert.deepEqual(calls, []);
    assert.equal(queue.length, 1);
    queue[0](0);
    assert.deepEqual(calls, []);
    assert.equal(queue.length, 2);
    queue[1](0);
    assert.deepEqual(calls, ["run"]);
    cancel();
  } finally {
    globalThis.requestAnimationFrame = originalRaf;
    globalThis.cancelAnimationFrame = originalCancel;
  }
});
