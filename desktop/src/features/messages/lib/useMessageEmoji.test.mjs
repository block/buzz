import assert from "node:assert/strict";
import test from "node:test";

import { customEmojiFromTags } from "@/shared/api/customEmoji.ts";
import { isEmojiOnlyMessage } from "@/shared/lib/emojiOnly.ts";

// useMessageEmoji is a thin React-hook wrapper around these two pure
// functions (memoized with React.useMemo). Exercise the underlying logic
// directly here — the same data both MessageRow (channels) and
// InboxMessageRow (inbox) now derive their emoji rendering from, so a custom
// emoji tag on an event must produce the same `customEmoji`/`emojiOnly`
// regardless of which row reads it.
const EMOJI_TAGS = [["emoji", "buzz", "https://relay/buzz.png"]];

test("derives custom emoji and emoji-only flag from event tags", () => {
  const customEmoji = customEmojiFromTags(EMOJI_TAGS);
  assert.deepEqual(customEmoji, [
    { shortcode: "buzz", url: "https://relay/buzz.png" },
  ]);
  assert.equal(isEmojiOnlyMessage(":buzz:", customEmoji), true);
  assert.equal(isEmojiOnlyMessage("hi :buzz:", customEmoji), false);
});

test("messages without tags get no custom emoji and are never emoji-only", () => {
  const customEmoji = undefined;
  assert.equal(isEmojiOnlyMessage(":buzz:", customEmoji), false);
});
