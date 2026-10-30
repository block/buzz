import assert from "node:assert/strict";
import test from "node:test";

import {
  EMOJI_TOKEN_PATTERN,
  registerCustomEmojiMarkdownIt,
} from "./customEmojiNode.ts";

// The input rule matches a completed `:name:` with one fixed pattern, however
// many emoji exist, then asks `resolveUrl` whether the name is known.

const tokenAtEnd = new RegExp(`(${EMOJI_TOKEN_PATTERN})$`);

test("the token pattern does not depend on the catalog", () => {
  assert.ok(EMOJI_TOKEN_PATTERN.length < 64);
  assert.equal(tokenAtEnd.exec("hello :party_parrot:")[1], ":party_parrot:");
  assert.equal(tokenAtEnd.exec(":WAVE:")[1], ":WAVE:");
});

test("the token pattern ignores unclosed names and names not at the end", () => {
  assert.equal(tokenAtEnd.exec(":party_parro"), null);
  assert.equal(tokenAtEnd.exec(":wave: trailing"), null);
  assert.equal(tokenAtEnd.exec("::"), null);
  assert.equal(tokenAtEnd.exec(`:${"a".repeat(65)}:`), null);
});

// ── markdown-it inline rule: word-boundary behavior ──────────────────────
// These drive the *actual* rule registered by registerCustomEmojiMarkdownIt
// (no reimplementation), using a minimal fake markdown-it that captures the
// rule and a minimal `state` shaped like markdown-it's inline state. The guard
// added for the edit-composer parse path must not fire mid-word or inside URLs.
function captureRule(shortcodes) {
  let captured = null;
  const md = {
    renderer: { rules: {} },
    inline: {
      ruler: {
        before(_anchor, _name, fn) {
          captured = fn;
        },
      },
    },
    utils: { escapeHtml: (s) => s },
  };
  registerCustomEmojiMarkdownIt(md, {
    resolveUrl: (sc) =>
      shortcodes.includes(sc) ? `https://b/${sc}.png` : undefined,
  });
  return captured;
}

// Run the rule at `pos`; return whether it matched and how far it advanced.
function runRule(rule, src, pos) {
  const state = {
    src,
    pos,
    push: () => ({ meta: {} }),
  };
  const matched = rule(state, false);
  return { matched, advanced: state.pos - pos };
}

test("rule fires for a boundary :shortcode: (start of string)", () => {
  const rule = captureRule(["buzz"]);
  const { matched, advanced } = runRule(rule, ":buzz:", 0);
  assert.equal(matched, true);
  assert.equal(advanced, ":buzz:".length);
});

test("rule fires for a :shortcode: preceded by whitespace", () => {
  const rule = captureRule(["buzz"]);
  // pos points at the `:` after the space.
  const { matched } = runRule(rule, "hi :buzz:", 3);
  assert.equal(matched, true);
});

test("rule does NOT fire when the colon is glued to a word char (not:buzz:)", () => {
  const rule = captureRule(["buzz"]);
  // pos points at the `:` immediately after `not`.
  const { matched } = runRule(rule, "not:buzz:", 3);
  assert.equal(matched, false);
});

test("rule does NOT fire inside a URL-like sequence (http://x:y:buzz:)", () => {
  const rule = captureRule(["buzz"]);
  const src = "http://x:y:buzz:";
  // pos points at the `:` immediately after `y` (a word char).
  const { matched } = runRule(rule, src, src.indexOf(":buzz:"));
  assert.equal(matched, false);
});

test("rule fires after punctuation boundary (e.g. parenthesis)", () => {
  const rule = captureRule(["buzz"]);
  // `(` is not a word char, so a `:shortcode:` after it still materializes.
  const { matched } = runRule(rule, "(:buzz:)", 1);
  assert.equal(matched, true);
});

test("rule leaves a name that is not cached as plain text", () => {
  const rule = captureRule(["buzz"]);
  const { matched, advanced } = runRule(rule, ":ghost:", 0);
  assert.equal(matched, false);
  assert.equal(advanced, 0);
});
