import assert from "node:assert/strict";
import test from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { clearMarkdownNodeCache, renderCachedMarkdown } from "./nodeCache.ts";

// The whole point of the cache is element-identity reuse across the message
// timeline's per-channel-switch remount: same parse inputs must return the
// SAME element (no re-parse), and anything that changes the parse output
// must miss.

const BASE = {
  components: {},
  content: "**bold** and `code`",
  variant: "i",
};

test("same parse inputs return the identical cached element", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({ ...BASE });
  const second = renderCachedMarkdown({ ...BASE });
  assert.equal(first, second);
  assert.match(renderToStaticMarkup(first), /<strong>bold<\/strong>/);
});

test("content changes miss the cache", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({ ...BASE });
  const second = renderCachedMarkdown({ ...BASE, content: "**bald**" });
  assert.notEqual(first, second);
});

test("customEmoji is keyed by value, not identity", () => {
  clearMarkdownNodeCache();
  const emoji = [{ shortcode: "buzz", url: "https://relay/buzz.png" }];
  const first = renderCachedMarkdown({
    ...BASE,
    content: "hi :buzz:",
    customEmoji: emoji,
  });
  // Fresh array, same values — the exact remount scenario (useMessageEmoji
  // rebuilds the array): must HIT.
  const second = renderCachedMarkdown({
    ...BASE,
    content: "hi :buzz:",
    customEmoji: [{ shortcode: "buzz", url: "https://relay/buzz.png" }],
  });
  assert.equal(first, second);
  // Same content, different emoji set (e.g. emoji added while editing —
  // custom-emoji.spec.ts Bug 2): must MISS so the new emoji renders.
  const third = renderCachedMarkdown({
    ...BASE,
    content: "hi :buzz:",
    customEmoji: [{ shortcode: "buzz", url: "https://relay/other.png" }],
  });
  assert.notEqual(first, third);
});

test("mention and channel names are part of the key", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({
    ...BASE,
    content: "ping @alice in #general",
    mentionNames: ["alice"],
    channelNames: ["general"],
  });
  const second = renderCachedMarkdown({
    ...BASE,
    content: "ping @alice in #general",
    mentionNames: ["alice", "bob"],
    channelNames: ["general"],
  });
  assert.notEqual(first, second);
});

test("render variants do not collide", () => {
  clearMarkdownNodeCache();
  const interactive = renderCachedMarkdown({ ...BASE });
  const nonInteractive = renderCachedMarkdown({ ...BASE, variant: "" });
  assert.notEqual(interactive, nonInteractive);
});

test("crafted values cannot forge key-segment boundaries", () => {
  clearMarkdownNodeCache();
  // Length-prefixed segments: a single name containing arbitrary bytes must
  // never be key-identical to two separate names, and values must not bleed
  // across the mention/channel field boundary.
  const joined = renderCachedMarkdown({
    ...BASE,
    mentionNames: ["ab"],
  });
  const split = renderCachedMarkdown({
    ...BASE,
    mentionNames: ["a", "b"],
  });
  assert.notEqual(joined, split);

  const inMentions = renderCachedMarkdown({ ...BASE, mentionNames: ["x"] });
  const inChannels = renderCachedMarkdown({ ...BASE, channelNames: ["x"] });
  assert.notEqual(inMentions, inChannels);
});

test("oversized content bypasses the cache", () => {
  clearMarkdownNodeCache();
  const huge = { ...BASE, content: "a".repeat(40_000) };
  const first = renderCachedMarkdown(huge);
  const second = renderCachedMarkdown(huge);
  assert.notEqual(first, second);
});

test("active search queries bypass the cache", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({ ...BASE, searchQuery: "bold" });
  const second = renderCachedMarkdown({ ...BASE, searchQuery: "bold" });
  assert.notEqual(first, second);
});
