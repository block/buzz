import assert from "node:assert/strict";
import { test } from "node:test";

import { resolveFileCard } from "./markdownFileCard.ts";

// A generic-file URL (non-media extension) does not match the relay-media
// proxy regex, so `rewriteRelayUrl` passes it through unchanged — assertions
// can compare hrefs directly.
const PDF_URL = `https://relay.example/media/${"a".repeat(64)}.pdf`;

test("resolveFileCard: returns null when there is no imeta entry", () => {
  assert.equal(resolveFileCard(undefined, PDF_URL, ""), null);
});

test("resolveFileCard: returns null without an href", () => {
  assert.equal(
    resolveFileCard({ m: "application/pdf" }, undefined, "doc"),
    null,
  );
});

test("resolveFileCard: returns null for image MIME (handled by img renderer)", () => {
  assert.equal(
    resolveFileCard({ m: "image/png" }, "https://b/x.png", ""),
    null,
  );
});

test("resolveFileCard: returns null for video MIME (handled by img renderer)", () => {
  assert.equal(
    resolveFileCard({ m: "video/mp4" }, "https://b/x.mp4", ""),
    null,
  );
});

test("resolveFileCard: returns null when imeta entry has no MIME", () => {
  assert.equal(resolveFileCard({ size: 10 }, PDF_URL, ""), null);
});

test("resolveFileCard: builds a card for a generic file, preferring imeta filename", () => {
  const card = resolveFileCard(
    { m: "application/pdf", size: 2048, filename: "Q3-budget.pdf" },
    PDF_URL,
    "link text",
  );
  assert.deepEqual(card, {
    href: PDF_URL,
    filename: "Q3-budget.pdf",
    size: 2048,
  });
});

test("resolveFileCard: falls back to link child text when imeta has no filename", () => {
  const card = resolveFileCard(
    { m: "application/zip" },
    PDF_URL,
    "  archive.zip  ",
  );
  assert.equal(card?.filename, "archive.zip");
  assert.equal(card?.size, undefined);
});

test("resolveFileCard: falls back to URL tail when no filename or child text", () => {
  const card = resolveFileCard({ m: "application/octet-stream" }, PDF_URL, "");
  assert.equal(card?.filename, `${"a".repeat(64)}.pdf`);
});

test("resolveFileCard: octet-stream (no magic bytes) is treated as a file", () => {
  // Text/code/data upload with no magic signature — the Slack-like case.
  const url = `https://relay.example/media/${"b".repeat(64)}.txt`;
  const card = resolveFileCard(
    { m: "application/octet-stream", filename: "notes.txt" },
    url,
    "",
  );
  assert.equal(card?.filename, "notes.txt");
});
