import assert from "node:assert/strict";
import test from "node:test";

import {
  playgroundDeviceBezel,
  playgroundDeviceBezelOuterSize,
  playgroundDeviceFrameSize,
  playgroundDeviceNubGutter,
  readPlaygroundStageBounds,
  rotateBezelEdge,
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

test("native stage bounds come from the inner screen, not the bezel box", () => {
  const screen = {
    getBoundingClientRect: () => ({ x: 48, y: 80, width: 200, height: 100 }),
  };
  const frame = {
    getBoundingClientRect: () => ({ x: 20, y: 20, width: 260, height: 180 }),
  };
  const viewport = { width: 393, height: 852 };
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

test("native stage bounds never overlap playground chrome", () => {
  const host = {
    getBoundingClientRect: () => ({
      x: 0,
      y: 40,
      width: 400,
      height: 360,
      bottom: 400,
    }),
  };
  const overlapping = { getBoundingClientRect: () => ({ bottom: 72 }) };
  assert.deepEqual(readPlaygroundStageBounds(host, undefined, overlapping), {
    x: 0,
    y: 72,
    width: 400,
    height: 328,
  });
  assert.deepEqual(
    readPlaygroundStageBounds(host, { width: 393, height: 852 }, overlapping),
    { x: 0, y: 72, width: 393, height: 852 },
  );
  const flush = { getBoundingClientRect: () => ({ bottom: 40 }) };
  assert.deepEqual(readPlaygroundStageBounds(host, undefined, flush), {
    x: 0,
    y: 40,
    width: 400,
    height: 360,
  });
});
