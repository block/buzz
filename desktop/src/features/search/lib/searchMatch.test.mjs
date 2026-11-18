import assert from "node:assert/strict";
import test from "node:test";

import { buildSearchResultPreview, splitSearchMatches } from "./searchMatch.ts";

test("splitSearchMatches highlights every case-insensitive lexeme match", () => {
  assert.deepEqual(splitSearchMatches("Mentions and mentions", "mentions"), [
    { isMatch: true, key: "0-8", text: "Mentions" },
    { isMatch: false, key: "8-5", text: " and " },
    { isMatch: true, key: "13-8", text: "mentions" },
  ]);
});

test("splitSearchMatches normalizes punctuation into search lexemes", () => {
  assert.deepEqual(splitSearchMatches("foo bar release", "foo-bar"), [
    { isMatch: true, key: "0-3", text: "foo" },
    { isMatch: false, key: "3-1", text: " " },
    { isMatch: true, key: "4-3", text: "bar" },
    { isMatch: false, key: "7-8", text: " release" },
  ]);
});

test("splitSearchMatches keeps completed tokens on lexeme boundaries", () => {
  assert.deepEqual(
    splitSearchMatches("projectile notes about project planning", "project pl"),
    [
      {
        isMatch: false,
        key: "0-23",
        text: "projectile notes about ",
      },
      { isMatch: true, key: "23-7", text: "project" },
      { isMatch: false, key: "30-1", text: " " },
      { isMatch: true, key: "31-2", text: "pl" },
      { isMatch: false, key: "33-6", text: "anning" },
    ],
  );
});

test("splitSearchMatches preserves exact and prefix modes for a repeated term", () => {
  assert.deepEqual(splitSearchMatches("foo foobar", "foo foo"), [
    { isMatch: true, key: "0-3", text: "foo" },
    { isMatch: false, key: "3-1", text: " " },
    { isMatch: true, key: "4-3", text: "foo" },
    { isMatch: false, key: "7-3", text: "bar" },
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

test("splitSearchMatches keeps one-character prefixes on lexeme boundaries", () => {
  assert.deepEqual(splitSearchMatches("A plan", "a"), [
    { isMatch: true, key: "0-1", text: "A" },
    { isMatch: false, key: "1-5", text: " plan" },
  ]);
});

test("splitSearchMatches maps expanding lowercase prefixes to original spans", () => {
  assert.deepEqual(splitSearchMatches("İstanbul release", "İs"), [
    { isMatch: true, key: "0-2", text: "İs" },
    { isMatch: false, key: "2-14", text: "tanbul release" },
  ]);
  assert.deepEqual(splitSearchMatches("İstanbul release", "İst"), [
    { isMatch: true, key: "0-3", text: "İst" },
    { isMatch: false, key: "3-13", text: "anbul release" },
  ]);
});

test("splitSearchMatches does not split a character whose lowercase form expands", () => {
  assert.deepEqual(splitSearchMatches("İstanbul release", "i"), [
    { isMatch: true, key: "0-1", text: "İ" },
    { isMatch: false, key: "1-15", text: "stanbul release" },
  ]);
});

test("splitSearchMatches preserves UTF-16 boundaries for supplementary letters", () => {
  assert.deepEqual(splitSearchMatches("𐐀İstanbul release", "𐐨İs"), [
    { isMatch: true, key: "0-4", text: "𐐀İs" },
    { isMatch: false, key: "4-14", text: "tanbul release" },
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

test("buildSearchResultPreview ignores an invalid completed-token substring", () => {
  const content = `${"projectile filler ".repeat(20)}project planning release notes`;
  const preview = buildSearchResultPreview(content, "project pl", 80);

  assert.match(preview, /project planning/);
  assert.match(preview, /^\.\.\./);
});

test("buildSearchResultPreview keeps the existing leading excerpt without a match", () => {
  assert.equal(
    buildSearchResultPreview("abcdefghijklmnopqrstuvwxyz", "missing", 10),
    "abcdefg...",
  );
});
