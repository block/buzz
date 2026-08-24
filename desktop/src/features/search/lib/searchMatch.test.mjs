import assert from "node:assert/strict";
import test from "node:test";

import { buildSearchResultPreview, splitSearchMatches } from "./searchMatch.ts";

test("splitSearchMatches highlights every case-insensitive literal match", () => {
  assert.deepEqual(splitSearchMatches("Mentions and mentions", "mentions"), [
    { isMatch: true, key: "0-8", text: "Mentions" },
    { isMatch: false, key: "8-5", text: " and " },
    { isMatch: true, key: "13-8", text: "mentions" },
  ]);
});

test("splitSearchMatches treats regex punctuation literally", () => {
  assert.deepEqual(splitSearchMatches("Use C++ (not C)", "C++"), [
    { isMatch: false, key: "0-4", text: "Use " },
    { isMatch: true, key: "4-3", text: "C++" },
    { isMatch: false, key: "7-8", text: " (not C)" },
  ]);
});

test("splitSearchMatches highlights non-adjacent prefix-search terms", () => {
  assert.deepEqual(splitSearchMatches("agent status mentions", "agent ment"), [
    { isMatch: true, key: "0-5", text: "agent" },
    { isMatch: false, key: "5-8", text: " status " },
    { isMatch: true, key: "13-4", text: "ment" },
    { isMatch: false, key: "17-4", text: "ions" },
  ]);
});

test("splitSearchMatches supports one-character scoped search", () => {
  assert.deepEqual(splitSearchMatches("A plan", "a"), [
    { isMatch: true, key: "0-1", text: "A" },
    { isMatch: false, key: "1-3", text: " pl" },
    { isMatch: true, key: "4-1", text: "a" },
    { isMatch: false, key: "5-1", text: "n" },
  ]);
});

test("buildSearchResultPreview keeps a late match visible", () => {
  const content = `${"prefix ".repeat(30)}mentions appear here ${"suffix ".repeat(20)}`;
  const preview = buildSearchResultPreview(content, "mentions", 96);

  assert.equal(preview.length <= 96, true);
  assert.match(preview, /mentions/i);
  assert.match(preview, /^\.\.\./);
  assert.match(preview, /\.\.\.$/);
});

test("buildSearchResultPreview keeps the existing leading excerpt without a match", () => {
  assert.equal(
    buildSearchResultPreview("abcdefghijklmnopqrstuvwxyz", "missing", 10),
    "abcdefg...",
  );
});
