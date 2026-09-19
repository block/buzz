import assert from "node:assert/strict";
import test from "node:test";

import {
  PLAYGROUND_DESKTOP_UA,
  PLAYGROUND_DEVICES,
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  clampPlaygroundDeviceScale,
  playgroundDeviceViewport,
  playgroundUserAgent,
  scalePlaygroundDeviceViewport,
  stepPlaygroundDeviceScale,
} from "./devices.ts";
import { DESKTOP_STAGE_PRESETS } from "./types.ts";

test("device museum is the locked six-device set", () => {
  assert.deepEqual(
    PLAYGROUND_DEVICES.map((device) => [
      device.id,
      device.width,
      device.height,
    ]),
    [
      ["iphone-se", 375, 667],
      ["iphone-16", 393, 852],
      ["iphone-16-pro-max", 440, 956],
      ["pixel-8", 412, 915],
      ["ipad-mini", 744, 1133],
      ["ipad-pro-11", 834, 1210],
    ],
  );
  assert.deepEqual(
    playgroundDeviceViewport(PLAYGROUND_DEVICES[1], "landscape"),
    { width: 852, height: 393 },
  );
});

test("mobile viewport is the device CSS size in both orientations", () => {
  const iphone = PLAYGROUND_DEVICES.find((device) => device.id === "iphone-16");
  assert.deepEqual(playgroundDeviceViewport(iphone, "portrait"), {
    width: 393,
    height: 852,
  });
  assert.deepEqual(playgroundDeviceViewport(iphone, "landscape"), {
    width: 852,
    height: 393,
  });
});

test("mobile devices use a mobile or tablet UA; desktop stays desktop", () => {
  assert.match(PLAYGROUND_DESKTOP_UA, /Macintosh/);
  assert.equal(playgroundUserAgent("desktop"), PLAYGROUND_DESKTOP_UA);
  assert.equal(playgroundUserAgent("responsive"), PLAYGROUND_DESKTOP_UA);
  for (const device of PLAYGROUND_DEVICES) {
    const ua = playgroundUserAgent("mobile", device);
    assert.equal(ua, device.userAgent);
    assert.match(ua, /iPhone|Android|iPad/);
    assert.match(ua, /Mobile/);
  }
});

test("desktop stage presets are locked", () => {
  assert.deepEqual(
    [...DESKTOP_STAGE_PRESETS],
    [375, 390, 768, 1024, 1280, 1440],
  );
});

test("device museum scale is 50–200% in 25% steps, default 100%", () => {
  assert.equal(PLAYGROUND_DEVICE_SCALE_DEFAULT, 100);
  assert.equal(clampPlaygroundDeviceScale(100), 100);
  assert.equal(clampPlaygroundDeviceScale(40), 50);
  assert.equal(clampPlaygroundDeviceScale(210), 200);
  assert.equal(clampPlaygroundDeviceScale(112), 100);
  assert.equal(clampPlaygroundDeviceScale(113), 125);
  assert.equal(stepPlaygroundDeviceScale(100, 1), 125);
  assert.equal(stepPlaygroundDeviceScale(100, -1), 75);
  assert.equal(stepPlaygroundDeviceScale(50, -1), 50);
  assert.equal(stepPlaygroundDeviceScale(200, 1), 200);
  assert.deepEqual(
    scalePlaygroundDeviceViewport({ width: 400, height: 800 }, 50),
    { width: 200, height: 400 },
  );
  assert.deepEqual(
    scalePlaygroundDeviceViewport({ width: 393, height: 852 }, 100),
    { width: 393, height: 852 },
  );
  assert.deepEqual(
    scalePlaygroundDeviceViewport({ width: 400, height: 800 }, 150),
    { width: 600, height: 1200 },
  );
});
