import assert from "node:assert/strict";
import test from "node:test";

import { buildKnownShortcodeAlternation } from "./customEmojiNode.ts";

// The input rule converts a *completed* known `:shortcode:` into the atom
// node. `buildKnownShortcodeAlternation` produces the inner alternation; the
// rule wraps it as `(:(?:<alt>):)$`, case-insensitive. These tests exercise
// that wrapped pattern so they cover the actual matching behavior.

function ruleRegex(shortcodes) {
  const alt = buildKnownShortcodeAlternation(shortcodes);
  if (!alt) return null;
  return new RegExp(`(:(?:${alt}):)$`, "i");
}

test("returns null when there are no usable shortcodes", () => {
  assert.equal(buildKnownShortcodeAlternation([]), null);
  assert.equal(buildKnownShortcodeAlternation(["", "   "]), null);
});

test("matches a completed known shortcode at the input end", () => {
  const re = ruleRegex(["party_parrot"]);
  const m = re.exec("hello :party_parrot:");
  assert.ok(m);
  assert.equal(m[1], ":party_parrot:");
});

test("does not match an unknown shortcode", () => {
  const re = ruleRegex(["party_parrot"]);
  assert.equal(re.exec("typing :foo:"), null);
});

test("does not match an incomplete (unclosed) shortcode", () => {
  const re = ruleRegex(["party_parrot"]);
  // User mid-typing — no trailing colon yet.
  assert.equal(re.exec(":party_parro"), null);
});

test("only matches when the shortcode ends at the input position", () => {
  const re = ruleRegex(["wave"]);
  // `:wave:` is present but not at the end → the input rule shouldn't fire.
  assert.equal(re.exec(":wave: trailing"), null);
});

test("is case-insensitive on the shortcode", () => {
  const re = ruleRegex(["Wave"]);
  const m = re.exec(":WAVE:");
  assert.ok(m);
  assert.equal(m[1], ":WAVE:");
});

test("longest-first: a longer name is not shadowed by a shorter prefix", () => {
  const re = ruleRegex(["party", "party_parrot"]);
  const m = re.exec(":party_parrot:");
  assert.ok(m);
  // Must prefer the full `:party_parrot:`, not stop at `:party`.
  assert.equal(m[1], ":party_parrot:");
});

test("dedupes and ignores blank entries", () => {
  const alt = buildKnownShortcodeAlternation(["wave", "wave", "", "  "]);
  assert.equal(alt, "wave");
});

// ── markdown-it inline rule: word-boundary behavior ──────────────────────
// These drive the *actual* rule registered by registerCustomEmojiMarkdownIt
// (no reimplementation), using a minimal fake markdown-it that captures the
// rule and a minimal `state` shaped like markdown-it's inline state. The guard
// added for the edit-composer parse path must not fire mid-word or inside URLs.
import { registerCustomEmojiMarkdownIt } from "./customEmojiNode.ts";

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
    shortcodes: () => shortcodes,
    resolveUrl: (sc) => `https://b/${sc}.png`,
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
  const rule = captureRule(["sprout"]);
  const { matched, advanced } = runRule(rule, ":sprout:", 0);
  assert.equal(matched, true);
  assert.equal(advanced, ":sprout:".length);
});

test("rule fires for a :shortcode: preceded by whitespace", () => {
  const rule = captureRule(["sprout"]);
  // pos points at the `:` after the space.
  const { matched } = runRule(rule, "hi :sprout:", 3);
  assert.equal(matched, true);
});

test("rule does NOT fire when the colon is glued to a word char (not:sprout:)", () => {
  const rule = captureRule(["sprout"]);
  // pos points at the `:` immediately after `not`.
  const { matched } = runRule(rule, "not:sprout:", 3);
  assert.equal(matched, false);
});

test("rule does NOT fire inside a URL-like sequence (http://x:y:sprout:)", () => {
  const rule = captureRule(["sprout"]);
  const src = "http://x:y:sprout:";
  // pos points at the `:` immediately after `y` (a word char).
  const { matched } = runRule(rule, src, src.indexOf(":sprout:"));
  assert.equal(matched, false);
});

test("rule fires after punctuation boundary (e.g. parenthesis)", () => {
  const rule = captureRule(["sprout"]);
  // `(` is not a word char, so a `:shortcode:` after it still materializes.
  const { matched } = runRule(rule, "(:sprout:)", 1);
  assert.equal(matched, true);
});
