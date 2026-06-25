import assert from "node:assert/strict";
import test from "node:test";

import { resolveQuickReactionEmojis } from "./useQuickReactionEmojis.ts";

function entry(emoji) {
  return { emoji };
}

test("quick reactions backfill defaults after stale custom emoji", () => {
  assert.deepEqual(
    resolveQuickReactionEmojis(
      [
        entry(":gone_one:"),
        entry(":gone_two:"),
        entry(":gone_three:"),
        entry(":gone_four:"),
      ],
      4,
      [],
    ),
    ["👍", "❤️", "😂", "🎉"],
  );
});

test("quick reactions skip stale custom emoji before applying the limit", () => {
  assert.deepEqual(
    resolveQuickReactionEmojis(
      [
        entry(":gone_one:"),
        entry(":gone_two:"),
        entry(":gone_three:"),
        entry(":gone_four:"),
        entry(":shipit:"),
        entry("🔥"),
      ],
      4,
      [{ shortcode: "shipit", url: "https://relay/shipit.png" }],
    ),
    [":shipit:", "🔥", "👍", "❤️"],
  );
});
