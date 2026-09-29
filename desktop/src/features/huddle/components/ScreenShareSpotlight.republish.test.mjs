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
