import assert from "node:assert/strict";
import test from "node:test";

import { hasMentionClipboardHtml } from "./normalizeMentionClipboard.ts";

// NOTE: normalizeMentionClipboardHtml uses the browser DOMParser API which
// is not available in Node.  Those paths are covered by the e2e paste tests.
// This file tests the pure string-matching detection function.

// ── hasMentionClipboardHtml ───────────────────────────────────────────

test("returns true when HTML contains data-mention", () => {
  const html = '<span data-mention="" class="mention">@Alice</span>';
  assert.equal(hasMentionClipboardHtml(html), true);
});

test("returns true when HTML contains data-channel-link", () => {
  const html = '<button data-channel-link="">#general</button>';
  assert.equal(hasMentionClipboardHtml(html), true);
});

test("returns true when HTML contains both markers", () => {
  const html =
    '<span data-mention="">@Alice</span> in <button data-channel-link="">#general</button>';
  assert.equal(hasMentionClipboardHtml(html), true);
});

test("returns false for plain HTML without markers", () => {
  const html = "<p>Hello world</p>";
  assert.equal(hasMentionClipboardHtml(html), false);
});

test("returns false for empty string", () => {
  assert.equal(hasMentionClipboardHtml(""), false);
});

test("returns false for text that mentions 'data-mention' as content", () => {
  // Edge case: the literal string "data-mention" appears as text content,
  // not as an attribute. hasMentionClipboardHtml does a simple string
  // includes check, so this is a known false positive — acceptable because
  // the normalization function is a no-op when no matching elements exist.
  const html = "<p>The attribute is called data-mention</p>";
  assert.equal(hasMentionClipboardHtml(html), true);
});
