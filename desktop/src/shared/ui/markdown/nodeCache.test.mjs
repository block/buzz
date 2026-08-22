import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
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

test("leading inline content is inserted into the first prose-capable block", () => {
  const components = {
    span: ({ children, node: _node, ...props }) =>
      "data-leading-inline-content" in props
        ? React.createElement(
            "button",
            { "data-chip": "", type: "button" },
            "00:01",
          )
        : React.createElement("span", props, children),
  };

  for (const [content, pattern] of [
    ["plain note", /<p><button[^>]*>00:01<\/button>plain note<\/p>/],
    ["> quoted note", /<blockquote>\s*<p><button[^>]*>00:01<\/button>quoted/],
    ["- list note", /<li><button[^>]*>00:01<\/button>list note<\/li>/],
    ["# heading", /<h1><button[^>]*>00:01<\/button>heading<\/h1>/],
  ]) {
    const node = renderCachedMarkdown({
      ...BASE,
      components,
      content,
      leadingInlineContent: true,
      variant: "leading",
    });
    assert.match(renderToStaticMarkup(node), pattern);
  }
});

test("leading inline content falls back before code and media blocks", () => {
  const components = {
    span: ({ children, node: _node, ...props }) =>
      "data-leading-inline-content" in props
        ? React.createElement(
            "button",
            { "data-chip": "", type: "button" },
            "00:01",
          )
        : React.createElement("span", props, children),
  };

  for (const [content, blockPattern] of [
    ["```js\nconst answer = 42;\n```", "<pre"],
    ["![](https://example.com/review.png)", "<img"],
    ["```js\nconst answer = 42;\n```\n\nlater note", "<pre"],
    ["![](https://example.com/review.png)\n\nlater note", "<img"],
    ["> ```js\nconst answer = 42;\n```\n\n> later note", "<pre"],
    ["> ![](https://example.com/review.png)\n\n> later note", "<img"],
  ]) {
    const node = renderCachedMarkdown({
      ...BASE,
      components,
      content,
      leadingInlineContent: true,
      variant: "leading-fallback",
    });
    const html = renderToStaticMarkup(node);
    assert.match(html, /<p><button[^>]*>00:01<\/button><\/p>/);
    assert.ok(html.indexOf("<button") < html.indexOf(blockPattern));
  }
});

test("leading inline content stays on the outer tight-list item", () => {
  const components = {
    span: ({ children, node: _node, ...props }) =>
      "data-leading-inline-content" in props
        ? React.createElement(
            "button",
            { "data-chip": "", type: "button" },
            "00:01",
          )
        : React.createElement("span", props, children),
  };
  const node = renderCachedMarkdown({
    ...BASE,
    components,
    content: "- parent\n  - child",
    leadingInlineContent: true,
    variant: "leading-tight-nested-list",
  });

  assert.match(
    renderToStaticMarkup(node),
    /<li><button[^>]*>00:01<\/button>parent\s*<ul>\s*<li>child<\/li>/,
  );
});

test("leading inline content participates in the cache key", () => {
  clearMarkdownNodeCache();
  const withoutLeading = renderCachedMarkdown({ ...BASE });
  const withLeading = renderCachedMarkdown({
    ...BASE,
    leadingInlineContent: true,
  });
  assert.notEqual(withoutLeading, withLeading);
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

test("hardLineBreaks changes the parse and the cache key", () => {
  clearMarkdownNodeCache();
  const content = "hello\nworld";
  const withBreaks = renderCachedMarkdown({ ...BASE, content });
  const withoutBreaks = renderCachedMarkdown({
    ...BASE,
    content,
    hardLineBreaks: false,
  });
  assert.notEqual(withBreaks, withoutBreaks);
  assert.match(renderToStaticMarkup(withBreaks), /<br/i);
  assert.doesNotMatch(renderToStaticMarkup(withoutBreaks), /<br/i);
  const withoutBreaksAgain = renderCachedMarkdown({
    ...BASE,
    content,
    hardLineBreaks: false,
  });
  assert.equal(withoutBreaks, withoutBreaksAgain);
});

test("active search queries bypass the cache", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({ ...BASE, searchQuery: "bold" });
  const second = renderCachedMarkdown({ ...BASE, searchQuery: "bold" });
  assert.notEqual(first, second);
});
