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

import { resolveCustomEmojiTags } from "./customEmojiTags.ts";

test("resolveCustomEmojiTags asks the resolver once for every name the legacy list lacks", async () => {
  const asked = [];
  const resolver = {
    resolve: async (names) => {
      asked.push([...names]);
      return {
        found: new Map([
          ["bufo_joy", { shortcode: "bufo_joy", url: "https://r/b.png" }],
        ]),
        failed: false,
      };
    },
  };
  const { tags, lookupFailed } = await resolveCustomEmojiTags(
    ":shipit: :bufo_joy: :ghost: :bufo_joy: :Other:",
    EMOJI,
    resolver,
  );
  assert.deepEqual(asked, [["bufo_joy", "ghost", "other"]]);
  assert.deepEqual(tags, [
    ["emoji", "shipit", EMOJI.find((e) => e.shortcode === "shipit").url],
    ["emoji", "bufo_joy", "https://r/b.png"],
  ]);
  assert.equal(lookupFailed, false);
});

test("resolveCustomEmojiTags skips the resolver when the legacy list covers every name", async () => {
  const resolver = {
    resolve: async () => {
      throw new Error("must not be called");
    },
  };
  const { tags } = await resolveCustomEmojiTags(
    ":shipit: no emoji here",
    EMOJI,
    resolver,
  );
  assert.equal(tags.length, 1);
  assert.deepEqual(
    await resolveCustomEmojiTags("plain text", EMOJI, resolver),
    { tags: [], lookupFailed: false },
  );
});

test("resolveCustomEmojiTags flags a failed lookup and still tags known names", async () => {
  const resolver = {
    resolve: async () => ({ found: new Map(), failed: true }),
  };
  const { tags, lookupFailed } = await resolveCustomEmojiTags(
    ":shipit: :ghost:",
    EMOJI,
    resolver,
  );
  assert.equal(lookupFailed, true);
  assert.equal(tags.length, 1);
});
