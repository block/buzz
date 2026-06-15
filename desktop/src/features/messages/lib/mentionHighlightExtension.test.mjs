import assert from "node:assert/strict";
import test from "node:test";

import { Schema } from "@tiptap/pm/model";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import {
  buildHighlightPatterns,
  findMentionBackspaceDeleteRange,
  findMentionDeleteRangeBeforeCursor,
  findHighlightMatches,
} from "./mentionHighlightExtension.ts";

const schema = new Schema({
  nodes: {
    doc: { content: "paragraph+" },
    paragraph: {
      content: "text*",
      group: "block",
      parseDOM: [{ tag: "p" }],
      toDOM: () => ["p", 0],
    },
    text: { group: "inline" },
  },
  marks: {},
});

function textDoc(text) {
  return schema.node("doc", null, [
    schema.node("paragraph", null, text ? [schema.text(text)] : undefined),
  ]);
}

function mentionDeleteSpec(fromOffset, toOffset) {
  return { mentionDelete: { fromOffset, toOffset } };
}

// ── buildHighlightPatterns ────────────────────────────────────────────

test("returns empty array when no names or channels provided", () => {
  assert.deepEqual(buildHighlightPatterns([], []), []);
});

test("builds a single pattern for mentions only", () => {
  const patterns = buildHighlightPatterns(["alice"], []);
  assert.equal(patterns.length, 1);
});

test("builds a single pattern for channels only", () => {
  const patterns = buildHighlightPatterns([], ["general"]);
  assert.equal(patterns.length, 1);
});

test("builds two patterns when both names and channels provided", () => {
  const patterns = buildHighlightPatterns(["alice"], ["general"]);
  assert.equal(patterns.length, 2);
});

test("escapes regex special characters in names", () => {
  const patterns = buildHighlightPatterns(["alice (admin)"], []);
  // Should not throw when used as regex
  const matches = findHighlightMatches("@alice (admin) hello", patterns);
  assert.equal(matches.length, 1);
  assert.equal(matches[0].match, "@alice (admin)");
});

test("escapes regex special characters in channel names", () => {
  const patterns = buildHighlightPatterns([], ["c++ help"]);
  const matches = findHighlightMatches("#c++ help", patterns);
  assert.equal(matches.length, 1);
});

// ── findHighlightMatches — @mentions ──────────────────────────────────

test("matches @mention at start of text", () => {
  const patterns = buildHighlightPatterns(["alice"], []);
  const matches = findHighlightMatches("@alice hello", patterns);
  assert.equal(matches.length, 1);
  assert.equal(matches[0].match, "@alice");
  assert.equal(matches[0].from, 0);
  assert.equal(matches[0].to, 6);
});

test("matches @mention after whitespace", () => {
  const patterns = buildHighlightPatterns(["bob"], []);
  const matches = findHighlightMatches("hey @bob", patterns);
  assert.equal(matches.length, 1);
  assert.equal(matches[0].match, "@bob");
});

test("does not match @mention embedded in a word", () => {
  const patterns = buildHighlightPatterns(["bob"], []);
  const matches = findHighlightMatches("email@bob.com", patterns);
  assert.equal(matches.length, 0);
});

test("matches are case-insensitive", () => {
  const patterns = buildHighlightPatterns(["Alice"], []);
  const matches = findHighlightMatches("@alice @ALICE @Alice", patterns);
  assert.equal(matches.length, 3);
});

test("matches multiple different mentions in one string", () => {
  const patterns = buildHighlightPatterns(["alice", "bob"], []);
  const matches = findHighlightMatches("@alice and @bob", patterns);
  assert.equal(matches.length, 2);
  assert.equal(matches[0].match, "@alice");
  assert.equal(matches[1].match, "@bob");
});

test("longer names matched first (no partial overlap)", () => {
  const patterns = buildHighlightPatterns(["al", "alice"], []);
  const matches = findHighlightMatches("@alice", patterns);
  // Should match "alice" not just "al"
  assert.equal(matches.length, 1);
  assert.equal(matches[0].match, "@alice");
});

// ── findHighlightMatches — #channels ──────────────────────────────────

test("matches #channel at start of text", () => {
  const patterns = buildHighlightPatterns([], ["general"]);
  const matches = findHighlightMatches("#general is cool", patterns);
  assert.equal(matches.length, 1);
  assert.equal(matches[0].match, "#general");
});

test("matches #channel after whitespace", () => {
  const patterns = buildHighlightPatterns([], ["random"]);
  const matches = findHighlightMatches("check #random", patterns);
  assert.equal(matches.length, 1);
});

test("does not match #channel embedded in a word", () => {
  const patterns = buildHighlightPatterns([], ["foo"]);
  const matches = findHighlightMatches("bar#foo", patterns);
  assert.equal(matches.length, 0);
});

test("channel matches are case-insensitive", () => {
  const patterns = buildHighlightPatterns([], ["General"]);
  const matches = findHighlightMatches("#general #GENERAL", patterns);
  assert.equal(matches.length, 2);
});

// ── findHighlightMatches — mixed ──────────────────────────────────────

test("matches both @mentions and #channels in the same text", () => {
  const patterns = buildHighlightPatterns(["alice"], ["general"]);
  const matches = findHighlightMatches("@alice in #general", patterns);
  assert.equal(matches.length, 2);
});

test("returns empty array for text with no matches", () => {
  const patterns = buildHighlightPatterns(["alice"], ["general"]);
  const matches = findHighlightMatches("nothing here", patterns);
  assert.equal(matches.length, 0);
});

test("handles empty text", () => {
  const patterns = buildHighlightPatterns(["alice"], []);
  const matches = findHighlightMatches("", patterns);
  assert.equal(matches.length, 0);
});

test("handles empty patterns against non-empty text", () => {
  const matches = findHighlightMatches("@alice #general", []);
  assert.equal(matches.length, 0);
});

// ── Trailing word boundary regression tests ───────────────────────────

test("@Marge should NOT match inside @Margex (trailing word boundary)", () => {
  const patterns = buildHighlightPatterns(["Marge"], []);
  const matches = findHighlightMatches("@Margex", patterns);
  assert.equal(matches.length, 0);
});

test("#general should NOT match inside #generally (trailing word boundary)", () => {
  const patterns = buildHighlightPatterns([], ["general"]);
  const matches = findHighlightMatches("#generally", patterns);
  assert.equal(matches.length, 0);
});

// ── findMentionDeleteRangeBeforeCursor ────────────────────────────────

test("finds a mention delete range when cursor is at the end of a mention", () => {
  const doc = textDoc("Hey @alice ");
  const from = 5;
  const to = 11;
  const decorations = DecorationSet.create(doc, [
    Decoration.inline(
      from,
      to,
      { class: "mention-highlight" },
      mentionDeleteSpec(0, 0),
    ),
  ]);

  assert.deepEqual(findMentionDeleteRangeBeforeCursor(decorations, to), {
    from,
    to,
  });
});

test("finds a mention delete range when cursor is inside a mention", () => {
  const doc = textDoc("Hey @alice ");
  const from = 5;
  const to = 11;
  const decorations = DecorationSet.create(doc, [
    Decoration.inline(
      from,
      to,
      { class: "mention-highlight" },
      mentionDeleteSpec(0, 0),
    ),
  ]);

  assert.deepEqual(findMentionDeleteRangeBeforeCursor(decorations, 8), {
    from,
    to,
  });
});

test("does not delete a mention when cursor is before it or after its trailing space", () => {
  const doc = textDoc("Hey @alice ");
  const from = 5;
  const to = 11;
  const decorations = DecorationSet.create(doc, [
    Decoration.inline(
      from,
      to,
      { class: "mention-highlight" },
      mentionDeleteSpec(0, 0),
    ),
  ]);

  assert.equal(findMentionDeleteRangeBeforeCursor(decorations, from), null);
  assert.equal(findMentionDeleteRangeBeforeCursor(decorations, to + 1), null);
});

test("backspace range includes the separator space after a mention", () => {
  const doc = textDoc("Hey @alice ");
  const from = 5;
  const to = 11;
  const cursorAfterSpace = 12;
  const decorations = DecorationSet.create(doc, [
    Decoration.inline(
      from,
      to,
      { class: "mention-highlight" },
      mentionDeleteSpec(0, 0),
    ),
  ]);

  assert.deepEqual(
    findMentionBackspaceDeleteRange(doc, decorations, cursorAfterSpace),
    {
      from,
      to: cursorAfterSpace,
    },
  );
});

test("finds the full agent mention range from either split decoration", () => {
  const doc = textDoc("Ask @kit");
  const from = 5;
  const to = 9;
  const decorations = DecorationSet.create(doc, [
    Decoration.inline(
      from,
      from + 1,
      { class: "agent-mention-at-hidden" },
      mentionDeleteSpec(0, to - (from + 1)),
    ),
    Decoration.inline(
      from + 1,
      to,
      { class: "mention-highlight agent-mention-highlight" },
      mentionDeleteSpec(-1, 0),
    ),
  ]);

  assert.deepEqual(findMentionDeleteRangeBeforeCursor(decorations, from + 1), {
    from,
    to,
  });
  assert.deepEqual(findMentionDeleteRangeBeforeCursor(decorations, to), {
    from,
    to,
  });
});
