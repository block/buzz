import assert from "node:assert/strict";
import test from "node:test";

import remarkChannelLinks from "./remarkChannelLinks.ts";
import remarkMentions from "./remarkMentions.ts";

/**
 * These run the real plugins over a real mdast tree. The pattern-level tests in
 * `mentionPattern.test.mjs` strip the leading-boundary capture group by hand,
 * which cannot catch a factory that swallows the boundary character instead of
 * emitting it back as text — that only shows up in the tree.
 */

function paragraph(...children) {
  return { type: "root", children: [{ type: "paragraph", children }] };
}
function text(value) {
  return { type: "text", value };
}
function kids(tree) {
  return tree.children[0].children;
}
/** `[type, value]` for each child, so a swallowed space fails the assertion. */
function shape(tree) {
  return kids(tree).map((node) => [node.type, node.value]);
}

function runMentions(value, names = ["alice", "bob"]) {
  const tree = paragraph(text(value));
  remarkMentions({ mentionNames: names })(tree);
  return tree;
}

function runChannels(value, names = ["general"]) {
  const tree = paragraph(text(value));
  remarkChannelLinks({ channelNames: names })(tree);
  return tree;
}

test("the boundary character survives as text before the mention", () => {
  assert.deepEqual(shape(runMentions("hi @alice")), [
    ["text", "hi "],
    ["mention", "@alice"],
  ]);
});

test("adjacent mentions keep the space between them", () => {
  assert.deepEqual(shape(runMentions("@alice @bob")), [
    ["mention", "@alice"],
    ["text", " "],
    ["mention", "@bob"],
  ]);
});

test("a mention at the start of the text produces no empty leading node", () => {
  assert.deepEqual(shape(runMentions("@alice hi")), [
    ["mention", "@alice"],
    ["text", " hi"],
  ]);
});

test("an opening paren before a mention stays text", () => {
  // Team expansions render as `Team (@ana @bo)`, so `(` opens a mention.
  assert.deepEqual(shape(runMentions("Team (@alice)")), [
    ["text", "Team ("],
    ["mention", "@alice"],
    ["text", ")"],
  ]);
});

test("an email address is left entirely alone", () => {
  assert.deepEqual(shape(runMentions("mail bob@alice.dev now")), [
    ["text", "mail bob@alice.dev now"],
  ]);
});

test("a channel link keeps its preceding text", () => {
  assert.deepEqual(shape(runChannels("see #general")), [
    ["text", "see "],
    ["channel-link", "#general"],
  ]);
});

test("an opening paren does not open a channel link", () => {
  // The composer's channel highlighter accepts only start-of-text or
  // whitespace; rendered messages must not be more permissive, or `(#general)`
  // shows no chip while typing and turns into a link once sent.
  assert.deepEqual(shape(runChannels("ask in (#general)")), [
    ["text", "ask in (#general)"],
  ]);
});

test("the generic channel fallback also refuses a mid-word prefix", () => {
  assert.deepEqual(shape(runChannels("issue-42#general", [])), [
    ["text", "issue-42#general"],
  ]);
});

test("inline code is left untouched", () => {
  const tree = paragraph({ type: "inlineCode", value: "@alice" });
  remarkMentions({ mentionNames: ["alice"] })(tree);
  assert.deepEqual(shape(tree), [["inlineCode", "@alice"]]);
});
