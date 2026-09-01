import assert from "node:assert/strict";
import { test } from "node:test";

import {
  decodeMarkdownDocBytes,
  isMarkdownDocFilename,
  MAX_MARKDOWN_DOC_BYTES,
} from "./markdownDocFile.ts";

// ── isMarkdownDocFilename ─────────────────────────────────────────────────

test("isMarkdownDocFilename: accepts .md, .markdown, .mdx", () => {
  assert.equal(isMarkdownDocFilename("README.md"), true);
  assert.equal(isMarkdownDocFilename("notes.markdown"), true);
  assert.equal(isMarkdownDocFilename("page.mdx"), true);
});

test("isMarkdownDocFilename: case-insensitive and whitespace-tolerant", () => {
  assert.equal(isMarkdownDocFilename("PLAN.MD"), true);
  assert.equal(isMarkdownDocFilename("  design.Md  "), true);
});

test("isMarkdownDocFilename: rejects other extensions", () => {
  assert.equal(isMarkdownDocFilename("report.pdf"), false);
  assert.equal(isMarkdownDocFilename("archive.zip"), false);
  assert.equal(isMarkdownDocFilename("script.mjs"), false);
  // Extension must be a suffix with a stem, not the whole name.
  assert.equal(isMarkdownDocFilename(".md"), false);
  assert.equal(isMarkdownDocFilename(""), false);
});

test("isMarkdownDocFilename: does not match mid-name extensions", () => {
  assert.equal(isMarkdownDocFilename("notes.md.zip"), false);
  assert.equal(isMarkdownDocFilename("mdfile.txt"), false);
});

// ── decodeMarkdownDocBytes ────────────────────────────────────────────────

test("decodeMarkdownDocBytes: decodes UTF-8 text", () => {
  const bytes = new TextEncoder().encode("# Hello 🐝\n\n- item");
  assert.deepEqual(decodeMarkdownDocBytes(bytes), {
    kind: "ok",
    text: "# Hello 🐝\n\n- item",
  });
});

test("decodeMarkdownDocBytes: rejects oversized payloads", () => {
  const bytes = new Uint8Array(MAX_MARKDOWN_DOC_BYTES + 1);
  assert.deepEqual(decodeMarkdownDocBytes(bytes), { kind: "too-large" });
});

test("decodeMarkdownDocBytes: accepts a payload exactly at the cap", () => {
  const bytes = new Uint8Array(MAX_MARKDOWN_DOC_BYTES).fill(0x61);
  const result = decodeMarkdownDocBytes(bytes);
  assert.equal(result.kind, "ok");
});

test("decodeMarkdownDocBytes: strict decode reports binary content", () => {
  // 0xFF is never valid in UTF-8.
  const bytes = new Uint8Array([0x23, 0x20, 0xff, 0xfe, 0x00]);
  assert.deepEqual(decodeMarkdownDocBytes(bytes), { kind: "binary" });
});
