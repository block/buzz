import assert from "node:assert/strict";
import test from "node:test";

import { getMentionOffset, hasMention } from "./hasMention.ts";

test("matches a plain @mention", () => {
  assert.equal(hasMention("hey @Fizz can you help", "Fizz"), true);
});

test("matches a mention at end of string", () => {
  assert.equal(hasMention("ping @Fizz", "Fizz"), true);
});

test("matches a mention immediately followed by Hangul (no space)", () => {
  // Regression: the trailing boundary previously required whitespace or ASCII
  // punctuation, so `@Fizz` butted against Korean text dropped the p-tag.
  assert.equal(hasMention("@Fizz이렇게됨", "Fizz"), true);
});

test("matches a mention preceded by Hangul (no space)", () => {
  assert.equal(hasMention("안녕@Fizz", "Fizz"), true);
});

test("matches a mention wrapped by Hangul on both sides", () => {
  assert.equal(hasMention("그래서@Fizz한테", "Fizz"), true);
});

test("matches a mention immediately followed by a CJK ideograph", () => {
  assert.equal(hasMention("@Fizz你好", "Fizz"), true);
});

test("matches a mention immediately followed by Kana", () => {
  assert.equal(hasMention("@Fizzこんにちは", "Fizz"), true);
});

test("does not match when a Latin word follows the name", () => {
  // `@Fizzbar` must NOT resolve to `@Fizz` — Latin letters are not a boundary.
  assert.equal(hasMention("@Fizzbar", "Fizz"), false);
});

test("reports the correct offset for a Hangul-adjacent mention", () => {
  const text = "블라 @Fizz이렇게";
  const offset = getMentionOffset(text, "Fizz");
  assert.notEqual(offset, null);
  assert.equal(text.slice(offset, offset + 5), "@Fizz");
});
