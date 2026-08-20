import assert from "node:assert/strict";
import test from "node:test";

import { buildCustomEmojiTags } from "./customEmojiTags.ts";

const EMOJI = [
  { shortcode: "party_parrot", url: "https://relay/p.gif" },
  { shortcode: "shipit", url: "https://relay/s.png" },
];

test("emits emoji tags only for known shortcodes present in content", () => {
  const tags = buildCustomEmojiTags("ship it :shipit: now", EMOJI);
  assert.deepEqual(tags, [["emoji", "shipit", "https://relay/s.png"]]);
});

test("ignores unknown shortcodes", () => {
  assert.deepEqual(buildCustomEmojiTags("hello :thinking: world", EMOJI), []);
});

test("dedupes repeated shortcodes, preserves first-appearance order", () => {
  const tags = buildCustomEmojiTags(":shipit: :party_parrot: :shipit:", EMOJI);
  assert.deepEqual(tags, [
    ["emoji", "shipit", "https://relay/s.png"],
    ["emoji", "party_parrot", "https://relay/p.gif"],
  ]);
});

test("empty emoji list yields no tags", () => {
  assert.deepEqual(buildCustomEmojiTags(":shipit:", []), []);
});

test("no shortcodes in content yields no tags", () => {
  assert.deepEqual(buildCustomEmojiTags("just plain text", EMOJI), []);
});
