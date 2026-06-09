import assert from "node:assert/strict";
import test from "node:test";

import { emojiDisplayName } from "./emojiName.ts";

test("emojiDisplayName resolves native emoji to emoji-mart shortcodes", () => {
  assert.equal(emojiDisplayName("🔥"), ":fire:");
  assert.equal(emojiDisplayName("😍"), ":heart_eyes:");
});

test("emojiDisplayName resolves skin-tone and ZWJ variants to shortcodes", () => {
  assert.equal(emojiDisplayName("👍🏽"), ":+1:");
  assert.equal(emojiDisplayName("👨‍👩‍👧‍👦"), ":man-woman-girl-boy:");
  assert.equal(emojiDisplayName("❤️"), ":heart:");
});

test("emojiDisplayName preserves custom emoji shortcodes", () => {
  assert.equal(emojiDisplayName(":party_parrot:"), ":party_parrot:");
});
