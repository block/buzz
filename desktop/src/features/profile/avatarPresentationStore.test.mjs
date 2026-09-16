import assert from "node:assert/strict";
import test from "node:test";

import {
  beginAvatarPresentation,
  getAvatarPresentation,
  resetAvatarPresentations,
} from "./avatarPresentationStore.ts";
import { buildAnimatedAvatarUrl } from "../../shared/lib/animatedAvatar.ts";

const POSTER_URL = "https://media.example.com/avatar-poster.png";
const ANIMATION_URL = "https://media.example.com/avatar-animation.png";

test("animated avatar presentation waits for both remote assets", async (t) => {
  const originalImage = globalThis.Image;
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
  const originalWindow = globalThis.window;
  const originalCreateObjectURL = URL.createObjectURL;
  const originalRevokeObjectURL = URL.revokeObjectURL;
  const requestedPaths = [];

  class ProbeImage {
    onerror = null;
    onload = null;
    referrerPolicy = "";

    set src(value) {
      const path = new URL(value).pathname;
      requestedPaths.push(path);
      queueMicrotask(() => {
        if (path === "/avatar-poster.png") this.onload?.();
        else this.onerror?.();
      });
    }
  }

  globalThis.Image = ProbeImage;
  globalThis.requestAnimationFrame = (callback) =>
    setTimeout(() => callback(performance.now()), 0);
  globalThis.window = globalThis;
  URL.createObjectURL = () => "blob:local-poster";
  URL.revokeObjectURL = () => {};
  t.after(() => {
    resetAvatarPresentations();
    globalThis.Image = originalImage;
    globalThis.requestAnimationFrame = originalRequestAnimationFrame;
    globalThis.window = originalWindow;
    URL.createObjectURL = originalCreateObjectURL;
    URL.revokeObjectURL = originalRevokeObjectURL;
  });

  const avatarUrl = buildAnimatedAvatarUrl(POSTER_URL, ANIMATION_URL);
  beginAvatarPresentation(avatarUrl, new Blob(["poster"]));

  await assert.doesNotReject(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (requestedPaths.length >= 2) return;
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    throw new Error("avatar presentation did not probe both assets");
  });

  assert.deepEqual(requestedPaths.slice(0, 2).sort(), [
    "/avatar-animation.png",
    "/avatar-poster.png",
  ]);
  assert.deepEqual(getAvatarPresentation(avatarUrl), {
    displayUrl: "blob:local-poster",
    state: "pending",
  });
});
