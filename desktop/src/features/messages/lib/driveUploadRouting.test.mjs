import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DRIVE_UPLOAD_THRESHOLD_BYTES,
  isAudioFile,
  uploadRouteFor,
} from "./driveUploadRouting.mjs";

const MB = 1024 * 1024;

// --- the size rule ----------------------------------------------------------

test("a small ordinary file stays on the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "notes.pdf",
      type: "application/pdf",
      sizeBytes: 2 * MB,
    }),
    "relay",
  );
});

test("anything over the threshold goes to Drive", () => {
  assert.equal(
    uploadRouteFor({
      name: "deck.pptx",
      type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      sizeBytes: DRIVE_UPLOAD_THRESHOLD_BYTES + 1,
    }),
    "drive",
  );
});

test("exactly the threshold stays on the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "deck.pptx",
      type: "application/pdf",
      sizeBytes: DRIVE_UPLOAD_THRESHOLD_BYTES,
    }),
    "relay",
  );
});

test("the threshold is the point Drive's simple upload stops working", () => {
  assert.equal(DRIVE_UPLOAD_THRESHOLD_BYTES, 5 * MB);
});

// --- the media rule ---------------------------------------------------------

test("a tiny video still goes to Drive", () => {
  assert.equal(
    uploadRouteFor({
      name: "clip.mp4",
      type: "video/mp4",
      sizeBytes: 1024,
      isVideo: true,
    }),
    "drive",
  );
});

test("a tiny audio file still goes to Drive", () => {
  assert.equal(
    uploadRouteFor({ name: "note.m4a", type: "audio/mp4", sizeBytes: 1024 }),
    "drive",
  );
});

test("audio is detected by extension when the MIME type says nothing", () => {
  assert.ok(isAudioFile({ name: "voice.mp3", type: "" }));
  assert.ok(
    isAudioFile({ name: "voice.wav", type: "application/octet-stream" }),
  );
  assert.ok(isAudioFile({ name: "voice.OGG" }));
});

test("a concrete MIME type outranks a misleading extension", () => {
  // A PDF named .mp3 is a PDF, and small enough to stay on the relay.
  assert.ok(!isAudioFile({ name: "report.mp3", type: "application/pdf" }));
  assert.equal(
    uploadRouteFor({
      name: "report.mp3",
      type: "application/pdf",
      sizeBytes: 1024,
    }),
    "relay",
  );
});

test("a file with no extension and no MIME type is not audio", () => {
  assert.ok(!isAudioFile({ name: "README" }));
  assert.ok(!isAudioFile({}));
});

test("an image under the threshold still goes to the relay", () => {
  assert.equal(
    uploadRouteFor({
      name: "screenshot.png",
      type: "image/png",
      sizeBytes: 2048,
    }),
    "relay",
  );
});

test("a missing or unparseable size does not force the Drive path", () => {
  assert.equal(uploadRouteFor({ name: "a.txt", type: "text/plain" }), "relay");
  assert.equal(
    uploadRouteFor({
      name: "a.txt",
      type: "text/plain",
      sizeBytes: Number.NaN,
    }),
    "relay",
  );
});
