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

// --- KaTeX math rendering (M1) ---

test("inline math renders through KaTeX", () => {
  clearMarkdownNodeCache();
  const el = renderCachedMarkdown({ ...BASE, content: "Energy $E=mc^2$!" });
  const html = renderToStaticMarkup(el);
  assert.match(html, /katex/);
});

test("display math renders through KaTeX as a block", () => {
  clearMarkdownNodeCache();
  const el = renderCachedMarkdown({
    ...BASE,
    content: "$$\n\\int_0^1 x^2\\, dx\n$$",
  });
  const html = renderToStaticMarkup(el);
  assert.match(html, /katex-display/);
});

test("same math parse inputs return the identical cached element", () => {
  clearMarkdownNodeCache();
  const first = renderCachedMarkdown({ ...BASE, content: "$E=mc^2$" });
  const second = renderCachedMarkdown({ ...BASE, content: "$E=mc^2$" });
  assert.equal(first, second);
  // Cache hit means the KaTeX output tree is reused, never re-parsed.
});

test("non-math messages still render and produce no katex (regression)", () => {
  clearMarkdownNodeCache();
  const el = renderCachedMarkdown({ ...BASE });
  const html = renderToStaticMarkup(el);
  assert.match(html, /<strong>bold<\/strong>/);
  assert.ok(!/katex/.test(html), "pure text produces no katex output");
});

test("KaTeX parse failure degrades to source without throwing", () => {
  clearMarkdownNodeCache();
  // throwOnError:false must swallow invalid LaTeX rather than raise.
  const el = renderCachedMarkdown({
    ...BASE,
    content: "bad $\\notarealcommand{}$",
  });
  assert.doesNotThrow(() => renderToStaticMarkup(el));
});

test("formula-bomb message is math-disabled and renders literally", () => {
  clearMarkdownNodeCache();
  // > 50 formulas trips the per-message cap => math off, literal `$...$`.
  const many = Array.from({ length: 51 }, (_, i) => `$x_{${i}}$`).join(" ");
  const el = renderCachedMarkdown({ ...BASE, content: many });
  const html = renderToStaticMarkup(el);
  assert.ok(!/katex/.test(html), "math disabled => no katex output");
  assert.match(html, /\$x_\{0\}\$/);
});

test("a single oversized formula is neutralised; sibling math still renders", () => {
  clearMarkdownNodeCache();
  // Only the >2KB formula trips the per-formula cap => that one is escaped to
  // literal while the healthy `$y=1$` sibling still renders via KaTeX. This is
  // the end-to-end check that the backslash-escape is honoured by remark-math.
  const huge = "a".repeat(2_001);
  const el = renderCachedMarkdown({
    ...BASE,
    content: `$y=1$ plus $${huge}$`,
  });
  const html = renderToStaticMarkup(el);
  assert.match(html, /katex/, "healthy sibling formula still renders as math");
});
