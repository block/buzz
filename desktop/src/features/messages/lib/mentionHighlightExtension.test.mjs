import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

import {
  assignMentionHighlightNames,
  buildHighlightPatterns,
  createMentionCaretSettlement,
  findHighlightMatches,
  insertPosForMentionTextInput,
  mentionTextInputInsertPos,
  positionAfterArrowLeftThroughMentionSpace,
  selectionAfterMentionTrailingSpace,
  shouldAdvanceMentionCaret,
} from "./mentionHighlightExtension.ts";

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

test("matches the first mention in a parenthesized team expansion", () => {
  const patterns = buildHighlightPatterns(["Planner", "Builder"], []);
  const matches = findHighlightMatches(
    "Launch Team(@Planner @Builder)",
    patterns,
  );
  assert.deepEqual(
    matches.map((match) => match.match),
    ["@Planner", "@Builder"],
  );
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

const schema = getSchema([
  StarterKit.configure({
    heading: false,
    trailingNode: false,
    link: false,
  }),
]);
const paragraph = (...content) => schema.nodes.paragraph.create(null, content);
const text = (value) => schema.text(value);
const document = (...content) => schema.nodes.doc.create(null, content);

test("selectionAfterMentionTrailingSpace steps past the space after @Name", () => {
  const doc = document(paragraph(text("@quinn ")));
  const spacePos = 1 + "@quinn".length;
  assert.equal(selectionAfterMentionTrailingSpace(doc, spacePos), spacePos + 1);
  assert.equal(
    selectionAfterMentionTrailingSpace(doc, spacePos + 1),
    spacePos + 1,
  );
});

test("selectionAfterMentionTrailingSpace leaves a caret inside the mention name", () => {
  const doc = document(paragraph(text("@quinn ")));
  assert.equal(selectionAfterMentionTrailingSpace(doc, 4), 4);
});

test("selectionAfterMentionTrailingSpace does not move without a trailing space", () => {
  const doc = document(paragraph(text("@quinn")));
  const end = 1 + "@quinn".length;
  assert.equal(selectionAfterMentionTrailingSpace(doc, end), end);
});

test("shouldAdvanceMentionCaret restores a remap while this editor is settling", () => {
  assert.equal(
    shouldAdvanceMentionCaret({
      from: 7,
      next: 8,
      settling: true,
      docChanged: false,
    }),
    true,
  );
});

test("shouldAdvanceMentionCaret does not steal ArrowLeft after settlement is cancelled", () => {
  assert.equal(
    shouldAdvanceMentionCaret({
      from: 7,
      next: 8,
      settling: false,
      docChanged: false,
    }),
    false,
  );
});

test("shouldAdvanceMentionCaret still advances after a document change", () => {
  assert.equal(
    shouldAdvanceMentionCaret({
      from: 7,
      next: 8,
      settling: false,
      docChanged: true,
    }),
    true,
  );
});

test("createMentionCaretSettlement keeps two editors independent", () => {
  const composerA = createMentionCaretSettlement();
  const composerB = createMentionCaretSettlement();
  composerA.arm(8);
  assert.equal(composerB.peek(), null);
  composerB.arm(12);
  composerA.cancel();
  assert.equal(composerA.peek(), null);
  assert.equal(composerB.peek(), 12);
});

test("insertPosForMentionTextInput redirects a caret at the chip edge", () => {
  const doc = document(paragraph(text("@quinn ")));
  const spacePos = 1 + "@quinn".length;
  assert.equal(
    insertPosForMentionTextInput(doc, spacePos, spacePos),
    spacePos + 1,
  );
  assert.equal(
    insertPosForMentionTextInput(doc, spacePos + 1, spacePos + 1),
    null,
  );
});

test("insertPosForMentionTextInput keeps a selected trailing space", () => {
  const doc = document(paragraph(text("@quinn ")));
  const spacePos = 1 + "@quinn".length;
  assert.equal(
    insertPosForMentionTextInput(doc, spacePos, spacePos + 1),
    spacePos + 1,
  );
});

test("mentionTextInputInsertPos honors a deliberate caret after settlement", () => {
  const doc = document(paragraph(text("@bob ")));
  const spacePos = 1 + "@bob".length;
  assert.equal(mentionTextInputInsertPos(doc, spacePos, spacePos, false), null);
  assert.equal(
    mentionTextInputInsertPos(doc, spacePos, spacePos, true),
    spacePos + 1,
  );
});

test("positionAfterArrowLeftThroughMentionSpace steps onto the token end", () => {
  const doc = document(paragraph(text("@bob ")));
  const afterSpace = 1 + "@bob".length + 1;
  assert.equal(
    positionAfterArrowLeftThroughMentionSpace(doc, afterSpace),
    afterSpace - 1,
  );
  assert.equal(
    positionAfterArrowLeftThroughMentionSpace(doc, afterSpace - 1),
    null,
  );
});

test("assignMentionHighlightNames skips an unchanged list", () => {
  const storage = { names: ["bob"], agentNames: [], channelNames: [] };
  assert.equal(assignMentionHighlightNames(storage, ["bob"], [], []), false);
});

test("assignMentionHighlightNames updates when a new mention is added", () => {
  const storage = { names: ["bob"], agentNames: [], channelNames: [] };
  assert.equal(
    assignMentionHighlightNames(storage, ["bob", "quinn"], [], []),
    true,
  );
  assert.deepEqual(storage.names, ["bob", "quinn"]);
});
