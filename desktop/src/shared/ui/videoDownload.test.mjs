import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DEFAULT_VIDEO_FILENAME,
  resolveVideoDownloadFilename,
} from "./videoDownload.ts";

test("resolveVideoDownloadFilename: keeps a real imeta filename", () => {
  assert.equal(resolveVideoDownloadFilename("clip.webm"), "clip.webm");
});

test("resolveVideoDownloadFilename: trims surrounding whitespace", () => {
  assert.equal(resolveVideoDownloadFilename("  demo.mp4  "), "demo.mp4");
});

test("resolveVideoDownloadFilename: falls back when undefined", () => {
  assert.equal(resolveVideoDownloadFilename(undefined), DEFAULT_VIDEO_FILENAME);
});

test("resolveVideoDownloadFilename: falls back when empty / whitespace-only", () => {
  assert.equal(resolveVideoDownloadFilename(""), DEFAULT_VIDEO_FILENAME);
  assert.equal(resolveVideoDownloadFilename("   "), DEFAULT_VIDEO_FILENAME);
});
