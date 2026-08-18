import assert from "node:assert/strict";
import { test } from "node:test";

import {
  classifyAttachmentPreview,
  decodeTextPreview,
  MAX_TEXT_PREVIEW_BYTES,
} from "./attachmentPreview.ts";

test("classifies Markdown from its filename even with octet-stream MIME", () => {
  assert.deepEqual(
    classifyAttachmentPreview("README.MD", "application/octet-stream"),
    { kind: "markdown" },
  );
});

test("classifies plain text, data, and common source files", () => {
  assert.deepEqual(classifyAttachmentPreview("notes.txt"), { kind: "text" });
  assert.deepEqual(classifyAttachmentPreview("data.json"), {
    kind: "text",
    language: "json",
  });
  assert.deepEqual(classifyAttachmentPreview("table.csv"), {
    kind: "text",
    language: "csv",
  });
  assert.deepEqual(classifyAttachmentPreview("src/main.rs"), {
    kind: "text",
    language: "rust",
  });
  assert.deepEqual(classifyAttachmentPreview("Component.TSX"), {
    kind: "text",
    language: "tsx",
  });
  assert.deepEqual(classifyAttachmentPreview("App.vue"), {
    kind: "text",
    language: "vue",
  });
});

test("classifies extensionless source filenames", () => {
  assert.deepEqual(classifyAttachmentPreview("Dockerfile"), {
    kind: "text",
    language: "dockerfile",
  });
  assert.deepEqual(classifyAttachmentPreview("Makefile"), {
    kind: "text",
    language: "makefile",
  });
  assert.deepEqual(classifyAttachmentPreview(".gitignore"), {
    kind: "text",
    language: "git-commit",
  });
});

test("classifies PDFs by extension or MIME fallback", () => {
  assert.deepEqual(classifyAttachmentPreview("report.pdf"), { kind: "pdf" });
  assert.deepEqual(classifyAttachmentPreview("attachment", "application/pdf"), {
    kind: "pdf",
  });
});

test("uses a relay URL extension when a friendly label has none", () => {
  assert.deepEqual(
    classifyAttachmentPreview(
      "Quarterly report",
      undefined,
      `https://relay.example/media/${"a".repeat(64)}.pdf`,
    ),
    { kind: "pdf" },
  );
});

test("keeps arbitrary binary files download-only", () => {
  assert.deepEqual(classifyAttachmentPreview("archive.zip"), { kind: "none" });
  assert.deepEqual(classifyAttachmentPreview("program.exe"), { kind: "none" });
  assert.deepEqual(classifyAttachmentPreview("payload.bin", "text/plain"), {
    kind: "none",
  });
});

test("decodes UTF-8 text and rejects binary, invalid, and oversized payloads", () => {
  assert.equal(
    decodeTextPreview(new TextEncoder().encode("hello 世界")),
    "hello 世界",
  );
  assert.throws(() => decodeTextPreview(new Uint8Array([65, 0, 66])), /UTF-8/);
  assert.throws(() => decodeTextPreview(new Uint8Array([0xff, 0xfe])), /UTF-8/);
  assert.throws(
    () => decodeTextPreview(new Uint8Array(MAX_TEXT_PREVIEW_BYTES + 1)),
    /too large/,
  );
});
