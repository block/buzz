import assert from "node:assert/strict";
import test from "node:test";

import { listAvatarCameras } from "./animatedAvatarCapture.ts";

function mockMediaDevices(devices) {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      mediaDevices: {
        enumerateDevices: async () => devices,
      },
    },
  });
}

test("listAvatarCameras: preserves blank pre-permission camera labels", async () => {
  mockMediaDevices([
    {
      deviceId: "default",
      kind: "videoinput",
      label: "",
    },
    {
      deviceId: "continuity",
      kind: "videoinput",
      label: "",
    },
    {
      deviceId: "microphone",
      kind: "audioinput",
      label: "Studio Mic",
    },
  ]);

  const cameras = await listAvatarCameras();

  assert.deepEqual(cameras, [
    { deviceId: "default", label: "" },
    { deviceId: "continuity", label: "" },
  ]);
  assert.equal(
    cameras.some((device) => device.label.trim().length > 0),
    false,
  );
});
