import assert from "node:assert/strict";
import { test } from "node:test";

import { resolveFileCard, resolveSnapshotCard } from "./markdownFileCard.ts";

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

// ── resolveSnapshotCard ───────────────────────────────────────────────────────

const SHA256 = "a".repeat(64);
const JSON_URL = `https://relay.example/media/${SHA256}.json`;
const PNG_URL = `https://relay.example/media/${SHA256}.png`;

test("resolveSnapshotCard: .agent.json with sha256 returns snapshot card", () => {
  const card = resolveSnapshotCard(
    {
      m: "application/json",
      size: 1234,
      filename: "analyst.agent.json",
      x: SHA256,
    },
    JSON_URL,
    "",
  );
  assert.ok(card !== null, "expected a snapshot card");
  assert.equal(card.filename, "analyst.agent.json");
  assert.equal(card.sha256, SHA256);
  assert.equal(card.snapshotKind, "agent");
  assert.equal(card.size, 1234);
});

test("resolveSnapshotCard: .agent.png with image/png mime returns snapshot card", () => {
  const card = resolveSnapshotCard(
    { m: "image/png", size: 2048, filename: "analyst.agent.png", x: SHA256 },
    PNG_URL,
    "",
  );
  assert.ok(card !== null);
  assert.equal(card.filename, "analyst.agent.png");
  assert.equal(card.snapshotKind, "agent");
});

test("resolveSnapshotCard: .agent.json with octet-stream mime is eligible", () => {
  // JSON snapshots often arrive as application/octet-stream.
  const card = resolveSnapshotCard(
    {
      m: "application/octet-stream",
      size: 1234,
      filename: "analyst.agent.json",
      x: SHA256,
    },
    JSON_URL,
    "",
  );
  assert.ok(card !== null, "octet-stream JSON should be a snapshot candidate");
});

test("resolveSnapshotCard: .agent.png with wrong mime is rejected", () => {
  const card = resolveSnapshotCard(
    { m: "application/json", filename: "analyst.agent.png", x: SHA256 },
    PNG_URL,
    "",
  );
  assert.equal(card, null, "PNG with non-image MIME must be rejected");
});

test("resolveSnapshotCard: missing sha256 (no x field) returns null", () => {
  const card = resolveSnapshotCard(
    { m: "application/json", filename: "analyst.agent.json" },
    JSON_URL,
    "",
  );
  assert.equal(card, null, "no sha256 must not produce a snapshot card");
});

test("resolveSnapshotCard: plain .json is not a snapshot candidate", () => {
  const card = resolveSnapshotCard(
    { m: "application/json", filename: "data.json", x: SHA256 },
    JSON_URL,
    "",
  );
  assert.equal(card, null, "plain .json must not be a snapshot candidate");
});

test("resolveSnapshotCard: plain .png is not a snapshot candidate", () => {
  const card = resolveSnapshotCard(
    { m: "image/png", filename: "photo.png", x: SHA256 },
    PNG_URL,
    "",
  );
  assert.equal(card, null, "plain .png must not be a snapshot candidate");
});

test("resolveSnapshotCard: deceptive double-extension foo.agent.json.exe rejected", () => {
  const card = resolveSnapshotCard(
    {
      m: "application/octet-stream",
      filename: "foo.agent.json.exe",
      x: SHA256,
    },
    JSON_URL,
    "",
  );
  assert.equal(card, null, "deceptive .exe extension must be rejected");
});

test("resolveSnapshotCard: agent.json without leading dot is rejected", () => {
  // The file must end with .agent.json, not just contain "agent.json".
  const card = resolveSnapshotCard(
    { m: "application/json", filename: "agent.json", x: SHA256 },
    JSON_URL,
    "",
  );
  // "agent.json" does NOT end with ".agent.json" (no leading dot prefix)
  assert.equal(card, null, "agent.json without prefix must be rejected");
});

test("resolveSnapshotCard: falls back to URL filename when no imeta filename", () => {
  const url = `https://relay.example/media/${SHA256}.agent.json`;
  const card = resolveSnapshotCard(
    { m: "application/json", x: SHA256 },
    url,
    "",
  );
  assert.ok(card !== null, "should classify from URL filename");
  assert.ok(card.filename.endsWith(".agent.json"));
});

test("resolveSnapshotCard: generic FileCard not affected", () => {
  // A generic PDF should still return null from the snapshot resolver
  // (and fall through to resolveFileCard).
  const card = resolveSnapshotCard(
    { m: "application/pdf", filename: "report.pdf", x: SHA256 },
    JSON_URL,
    "",
  );
  assert.equal(card, null);
});
