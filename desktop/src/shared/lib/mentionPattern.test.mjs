import assert from "node:assert/strict";
import test from "node:test";

import { buildMentionPattern } from "./mentionPattern.ts";

function matchNames(text, names) {
  const pattern = buildMentionPattern(names);
  return [...text.matchAll(pattern)].map((m) => m[0]);
}

test("matches a known name followed by whitespace", () => {
  assert.deepEqual(matchNames("hi @Fizz there", ["Fizz"]), ["@Fizz"]);
});

test("matches a known name butted against Hangul", () => {
  // Regression: `@Fizz이렇게` failed the boundary and was not highlighted.
  assert.deepEqual(matchNames("@Fizz이렇게됨", ["Fizz"]), ["@Fizz"]);
});

test("matches a known name butted against a CJK ideograph", () => {
  assert.deepEqual(matchNames("@Fizz你好", ["Fizz"]), ["@Fizz"]);
});

test("does not match a longer Latin word as a partial name", () => {
  assert.deepEqual(matchNames("@Fizzbar", ["Fizz"]), []);
});

test("prefers the longest known name (longest-first)", () => {
  assert.deepEqual(matchNames("@Fizz Bee한테", ["Fizz", "Fizz Bee"]), [
    "@Fizz Bee",
  ]);
});

test("returns no matches when no known names are provided", () => {
  assert.deepEqual(matchNames("@Fizz이렇게", []), []);
});
