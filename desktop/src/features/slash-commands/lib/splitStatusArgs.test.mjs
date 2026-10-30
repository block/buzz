import assert from "node:assert/strict";
import test from "node:test";

import { splitStatusArgs } from "./splitStatusArgs.ts";
import { standardEmojiForShortcode } from "./standardEmojiShortcode.ts";

const sources = {
  customEmoji: [{ shortcode: "partyparrot" }, { shortcode: "tada" }],
  standardEmoji: standardEmojiForShortcode,
};

const rows = [
  ["🚀 shipping", "🚀", "shipping"],
  ["🚀", "🚀", ""],
  ["  🚀   shipping the thing  ", "🚀", "shipping the thing"],
  ["👨‍👩‍👧‍👦 family time", "👨‍👩‍👧‍👦", "family time"],
  ["🇺🇸 travelling", "🇺🇸", "travelling"],
  ["👍🏽 nice", "👍🏽", "nice"],
  ["❤️ love", "❤️", "love"],
  ["1️⃣ first", "1️⃣", "first"],
  ["#️⃣ hashtag", "#️⃣", "hashtag"],
  [":rocket: shipping", "🚀", "shipping"],
  [":ROCKET: shipping", "🚀", "shipping"],
  [":thumbsup: nice", "👍", "nice"],
  [":partyparrot: party", ":partyparrot:", "party"],
  [":PartyParrot: party", ":partyparrot:", "party"],
  [":tada: shipped", ":tada:", "shipped"],
  [":partyparrot:", ":partyparrot:", ""],
  ["🚀🚀 double", "", "🚀🚀 double"],
  ["🚀🔥 two glyphs", "", "🚀🔥 two glyphs"],
  ["1 thing left", "", "1 thing left"],
  ["© 2026", "", "© 2026"],
  ["a shipping", "", "a shipping"],
  [":nosuchemoji: shipping", "", ":nosuchemoji: shipping"],
  ["::: shipping", "", "::: shipping"],
  [":rocket:x shipping", "", ":rocket:x shipping"],
  ["in a meeting :rocket:", "", "in a meeting :rocket:"],
  ["shipping 🚀", "", "shipping 🚀"],
  ['"🚀" shipping', "", '"🚀" shipping'],
];

for (const [args, emoji, text] of rows) {
  test(`splits ${JSON.stringify(args)}`, () => {
    assert.deepEqual(splitStatusArgs(args, sources), { emoji, text });
  });
}

test("a standard shortcode resolves only when custom emoji do not claim it", () => {
  const standardOnly = { ...sources, customEmoji: [] };
  assert.deepEqual(splitStatusArgs(":tada: shipped", standardOnly), {
    emoji: "🎉",
    text: "shipped",
  });
});
