import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const src = fs.readFileSync(
  path.join(
    path.dirname(fileURLToPath(import.meta.url)),
    "useHuddleScreenShare.ts",
  ),
  "utf8",
);

test("startShare sets local preview before mint/connect", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  const acquireIdx = body.indexOf("await acquireDisplayMedia()");
  const previewIdx = body.indexOf("setLocalPreviewStream(acquired)");
  const mintIdx = body.indexOf("mintScreenShareToken");
  assert.ok(acquireIdx >= 0, "acquires display media");
  assert.ok(previewIdx > acquireIdx, "preview after acquire");
  assert.ok(mintIdx > previewIdx, "mint after preview");
  assert.match(body, /setSharing\(true\)/);
});

test("startShare clears preview on failure after acquire", () => {
  const start = src.indexOf("const startShare");
  const body = src.slice(start, src.indexOf("const stopShare", start));
  assert.match(body, /setLocalPreviewStream\(null\)/);
  assert.match(body, /stopMediaStreamTracks\(acquired\)/);
});
