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
