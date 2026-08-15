import assert from "node:assert/strict";
import { test } from "node:test";

import {
  decodeFileViewerContent,
  MAX_FILE_PREVIEW_BYTES,
} from "./fileViewerContent.ts";

const encode = (text) => new TextEncoder().encode(text);

test("UTF-8 text decodes, including multi-byte characters", () => {
  assert.deepEqual(decodeFileViewerContent(encode("# Titre\n\néàü 🐝")), {
    status: "text",
    text: "# Titre\n\néàü 🐝",
  });
});

test("empty file decodes to empty text rather than binary", () => {
  assert.deepEqual(decodeFileViewerContent(new Uint8Array(0)), {
    status: "text",
    text: "",
  });
});

// The imeta MIME is sender-controlled, so the binary decision must come from
// the bytes. Losing this check renders a blob as garbled text.
test("a NUL byte in the sniffed head marks the file binary", () => {
  const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x00, 0x1a, 0x0a]);
  assert.deepEqual(decodeFileViewerContent(bytes), { status: "binary" });
});

test("a NUL just inside the sniff window is still caught", () => {
  const bytes = new Uint8Array(9000).fill(0x61);
  bytes[8191] = 0x00;
  assert.deepEqual(decodeFileViewerContent(bytes), { status: "binary" });
});

// The sniff is bounded so a large text file stays cheap; a NUL past the window
// is accepted as text by design.
test("a NUL past the sniff window does not mark the file binary", () => {
  const bytes = new Uint8Array(9000).fill(0x61);
  bytes[8192] = 0x00;
  assert.equal(decodeFileViewerContent(bytes).status, "text");
});

test("bytes over the preview cap are reported too-large, not decoded", () => {
  const bytes = new Uint8Array(MAX_FILE_PREVIEW_BYTES + 1).fill(0x61);
  assert.deepEqual(decodeFileViewerContent(bytes), { status: "too-large" });
});

test("bytes exactly at the preview cap still decode", () => {
  const bytes = new Uint8Array(MAX_FILE_PREVIEW_BYTES).fill(0x61);
  assert.equal(decodeFileViewerContent(bytes).status, "text");
});
