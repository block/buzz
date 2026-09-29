import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const src = fs.readFileSync(
  new URL("./ScreenShareSpotlight.tsx", import.meta.url),
  "utf8",
);

test("ScreenShareSpotlight shows spinner overlay while republishing", () => {
  assert.match(src, /republishing/);
  assert.match(src, /huddle-share-republishing/);
  assert.match(src, /Reconnecting/);
  assert.match(src, /Spinner/);
});

test("ScreenShareSpotlight attaches LiveKit RemoteTrack for remote video", () => {
  assert.match(src, /videoTrack/);
  assert.match(src, /videoTrack\.attach\(el\)/);
  assert.match(src, /videoTrack\.detach\(el\)/);
  // Local preview still uses srcObject; remote must not rely on it alone.
  assert.match(src, /el\.srcObject = stream/);
});
