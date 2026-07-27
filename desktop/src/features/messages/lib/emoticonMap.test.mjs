import assert from "node:assert/strict";
import test from "node:test";

import { buildEmoticonFindPattern } from "./emoticonAutoReplace.ts";
import { EMOTICON_MAP } from "./emoticonMap.ts";

// These exercise the actual pattern the composer's auto-replace extension
// builds (emoticonAutoReplace.ts), not a reimplementation, so they cover the
// real word-boundary matching behavior.

test("every mapped emoticon matches when preceded by a space", () => {
  for (const key of Object.keys(EMOTICON_MAP)) {
    const m = buildEmoticonFindPattern(key).exec(`hello ${key}`);
    assert.ok(m, `expected ${key} to match after a space`);
    assert.equal(m[1], key);
  }
});

test("matches at the very start of input (no preceding character)", () => {
  const m = buildEmoticonFindPattern(":)").exec(":)");
  assert.ok(m);
  assert.equal(m[1], ":)");
});

test("does not match mid-word (no preceding space)", () => {
  assert.equal(buildEmoticonFindPattern(":)").exec("hi:)"), null);
  assert.equal(buildEmoticonFindPattern(":D").exec(":Dance"), null);
});

test("does not match an incomplete emoticon", () => {
  assert.equal(buildEmoticonFindPattern(":-)").exec("hello :-"), null);
  assert.equal(buildEmoticonFindPattern("<3").exec("hello <"), null);
});

test("maps common smileys to the expected emoji", () => {
  assert.equal(EMOTICON_MAP[":)"], "🙂");
  assert.equal(EMOTICON_MAP[":("], "🙁");
  assert.equal(EMOTICON_MAP[":D"], "😀");
  assert.equal(EMOTICON_MAP[";)"], "😉");
  assert.equal(EMOTICON_MAP[":P"], "😛");
  assert.equal(EMOTICON_MAP["<3"], "❤️");
});

test("every value is non-empty and every key non-blank", () => {
  for (const [key, value] of Object.entries(EMOTICON_MAP)) {
    assert.ok(key.trim().length > 0);
    assert.ok(value.trim().length > 0);
  }
});
