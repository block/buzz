import assert from "node:assert/strict";
import test from "node:test";

import { isVideoFile, videoMimeForFile } from "./videoFileType.ts";

// ── MIME type is authoritative when present ───────────────────────────

test("a video MIME type is detected as a video", () => {
  assert.equal(isVideoFile({ name: "clip.mp4", type: "video/mp4" }), true);
  assert.equal(
    videoMimeForFile({ name: "clip.mp4", type: "video/mp4" }),
    "video/mp4",
  );
});

test("an uppercase video MIME type is normalized", () => {
  assert.equal(
    videoMimeForFile({ name: "clip.MOV", type: "VIDEO/QUICKTIME" }),
    "video/quicktime",
  );
});

test("a non-video MIME type wins over a video extension", () => {
  // A GIF misnamed `.mp4` is still an image — never queue it as a video.
  assert.equal(isVideoFile({ name: "loop.mp4", type: "image/gif" }), false);
  assert.equal(
    videoMimeForFile({ name: "loop.mp4", type: "image/gif" }),
    undefined,
  );
});

test("an image is not a video", () => {
  assert.equal(isVideoFile({ name: "photo.png", type: "image/png" }), false);
});

// ── Extension fallback when the MIME type is missing or opaque ────────

test("an empty MIME type falls back to the filename extension", () => {
  assert.equal(isVideoFile({ name: "clip.mp4", type: "" }), true);
  assert.equal(videoMimeForFile({ name: "clip.mp4", type: "" }), "video/mp4");
});

test("an absent MIME type falls back to the filename extension", () => {
  assert.equal(videoMimeForFile({ name: "clip.webm" }), "video/webm");
});

test("an octet-stream MIME type falls back to the filename extension", () => {
  assert.equal(
    videoMimeForFile({ name: "clip.mov", type: "application/octet-stream" }),
    "video/quicktime",
  );
  assert.equal(
    videoMimeForFile({ name: "clip.mkv", type: "binary/octet-stream" }),
    "video/x-matroska",
  );
});

test("every supported video extension is recognized without a MIME type", () => {
  for (const [extension, mime] of [
    ["avi", "video/x-msvideo"],
    ["m4v", "video/mp4"],
    ["mkv", "video/x-matroska"],
    ["mov", "video/quicktime"],
    ["mp4", "video/mp4"],
    ["webm", "video/webm"],
  ]) {
    assert.equal(
      videoMimeForFile({ name: `clip.${extension}`, type: "" }),
      mime,
    );
  }
});

test("extension matching is case-insensitive", () => {
  assert.equal(videoMimeForFile({ name: "CLIP.MP4", type: "" }), "video/mp4");
});

test("a non-video extension with no MIME type is not a video", () => {
  assert.equal(isVideoFile({ name: "notes.txt", type: "" }), false);
  assert.equal(
    isVideoFile({ name: "archive.zip", type: "application/octet-stream" }),
    false,
  );
});

test("an extensionless file with no MIME type is not a video", () => {
  assert.equal(isVideoFile({ name: "clip", type: "" }), false);
  assert.equal(isVideoFile({ type: "" }), false);
});

test("a dotfile is not treated as having an extension", () => {
  // `.mp4` as an entire basename is a hidden file, not an mp4 named "".
  assert.equal(isVideoFile({ name: ".mp4", type: "" }), false);
});

test("a trailing dot is not treated as an extension", () => {
  assert.equal(isVideoFile({ name: "clip.", type: "" }), false);
});

test("only the last extension is consulted", () => {
  assert.equal(videoMimeForFile({ name: "clip.mp4.txt", type: "" }), undefined);
  assert.equal(
    videoMimeForFile({ name: "notes.txt.mp4", type: "" }),
    "video/mp4",
  );
});
