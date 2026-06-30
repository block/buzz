import assert from "node:assert/strict";
import test from "node:test";

import {
  formatDurationMs,
  formatTranscriptTime,
  getToolDurationDisplay,
  isInlineImageData,
  parseToolResultValue,
} from "./agentSessionUtils.ts";

// ---- isInlineImageData (dual-layer image-scheme security guard) ----

test("isInlineImageData accepts data:image/ URIs", () => {
  const dataUri =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgYGAAAAAEAAH2FzhVAAAAAElFTkSuQmCC";
  assert.equal(isInlineImageData(dataUri), true);
});

test("isInlineImageData rejects non-image data: schemes (no passthrough widening)", () => {
  // A non-image data: URI must NOT be treated as a safe inline image —
  // it has to fall through to the relay rewrite path.
  assert.equal(
    isInlineImageData(
      "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
    ),
    false,
  );
  assert.equal(isInlineImageData("data:application/json;base64,e30="), false);
});

test("isInlineImageData rejects relay-relative and absolute media URLs", () => {
  assert.equal(isInlineImageData("/media/abc123.png"), false);
  assert.equal(isInlineImageData("https://relay.example/media/abc.png"), false);
});

// ---- formatDurationMs ----

test("formatDurationMs returns null for negative input", () => {
  assert.equal(formatDurationMs(-1), null);
});

test("formatDurationMs renders sub-10s with one decimal", () => {
  assert.equal(formatDurationMs(400), "0.4s");
  assert.equal(formatDurationMs(9900), "9.9s");
});

test("formatDurationMs rounds 10s..60s to whole seconds", () => {
  assert.equal(formatDurationMs(12300), "12s");
  assert.equal(formatDurationMs(59400), "59s");
});

test("formatDurationMs renders minutes and seconds", () => {
  assert.equal(formatDurationMs(90000), "1m 30s");
  assert.equal(formatDurationMs(120000), "2m");
});

test("formatDurationMs carries a rounded 60s into the next minute", () => {
  // 89.7s rounds the seconds component to 60, which must carry to 1m 30s
  assert.equal(formatDurationMs(89700), "1m 30s");
});

test("formatTranscriptTime renders a short 12-hour time without seconds", () => {
  assert.equal(formatTranscriptTime("2026-06-30T17:00:02.000"), "5:00 PM");
});

// ---- parseToolResultValue (JSON double-parse) ----

test("parseToolResultValue returns null for empty/whitespace", () => {
  assert.equal(parseToolResultValue(""), null);
  assert.equal(parseToolResultValue("   "), null);
});

test("parseToolResultValue parses a JSON object", () => {
  assert.deepEqual(parseToolResultValue('{"duration_ms":123}'), {
    duration_ms: 123,
  });
});

test("parseToolResultValue unwraps a double-encoded JSON string", () => {
  // The result is a JSON string that itself contains JSON.
  const doubleEncoded = JSON.stringify(JSON.stringify({ ok: true }));
  assert.deepEqual(parseToolResultValue(doubleEncoded), { ok: true });
});

test("parseToolResultValue returns the inner string when it is not JSON", () => {
  const wrapped = JSON.stringify("plain text");
  assert.equal(parseToolResultValue(wrapped), "plain text");
});

test("parseToolResultValue returns null for invalid JSON", () => {
  assert.equal(parseToolResultValue("not json {"), null);
});

// ---- getToolDurationDisplay (fallback chain) ----

const startedAt = "2026-06-14T19:00:00.000Z";
const completedAt = "2026-06-14T19:00:02.000Z";

test("getToolDurationDisplay prefers start/complete timestamps", () => {
  assert.equal(
    getToolDurationDisplay({ startedAt, completedAt, result: "" }),
    "2.0s",
  );
});

test("getToolDurationDisplay falls back to duration_ms in the result payload", () => {
  assert.equal(
    getToolDurationDisplay({
      startedAt: null,
      completedAt: null,
      result: JSON.stringify({ duration_ms: 3500 }),
    }),
    "3.5s",
  );
});

test("getToolDurationDisplay falls back to elapsed_ms when duration_ms absent", () => {
  assert.equal(
    getToolDurationDisplay({
      result: JSON.stringify({ elapsed_ms: 65000 }),
    }),
    "1m 5s",
  );
});

test("getToolDurationDisplay returns null when no duration is available", () => {
  assert.equal(getToolDurationDisplay({ result: "" }), null);
  assert.equal(
    getToolDurationDisplay({ result: JSON.stringify({ other: 1 }) }),
    null,
  );
});
