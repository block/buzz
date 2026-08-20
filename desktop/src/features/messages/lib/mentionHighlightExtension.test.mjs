import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

import {
  agentMentionChipDeleteRange,
  buildHighlightPatterns,
  findAgentMentionRanges,
  findHighlightMatches,
  snapCaretOutOfAgentMention,
  withTrailingSpace,
} from "./mentionHighlightExtension.ts";

// Schema mirrors the composer for document-absolute position tests.
const schema = getSchema([
  StarterKit.configure({
    hardBreak: { keepMarks: true },
    heading: false,
    trailingNode: false,
    link: false,
  }),
]);
const para = (...c) => schema.nodes.paragraph.create(null, c);
const t = (s) => schema.text(s);
function doc(...content) {
  return schema.nodes.doc.create(null, content);
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

// ── Agent-mention caret snap (#2707) ──────────────────────────────────
// Persistent auto-tag keeps `@Agent ` at the head of the composer. Mentions
// are decorations (not atoms) and the `@` is width:0, so a click that looks
// like the start of the chip often lands *inside* the name. Typing there
// mutates the display name and drops the decoration.

test("findAgentMentionRanges locates agent mentions in document coords", () => {
  // PM: doc=0, para opens at 0, text starts at 1: "@Ada " is positions 1..5
  const d = doc(para(t("@Ada hello")));
  const ranges = findAgentMentionRanges(d, ["Ada"]);
  assert.equal(ranges.length, 1);
  assert.equal(ranges[0].from, 1);
  assert.equal(ranges[0].to, 5); // "@Ada"
});

test("snapCaretOutOfAgentMention leaves edge positions alone", () => {
  const d = doc(para(t("@Ada ")));
  // Positions: 1=before @, 5=after a, 6=after trailing space
  assert.equal(snapCaretOutOfAgentMention(d, 1, ["Ada"]), 1);
  assert.equal(snapCaretOutOfAgentMention(d, 5, ["Ada"]), 5);
  assert.equal(snapCaretOutOfAgentMention(d, 6, ["Ada"]), 6);
});

test("snapCaretOutOfAgentMention snaps interior caret past trailing space", () => {
  // "@Ada " — interior positions 2..4 (after @, mid-name) must jump to 6
  // (after the trailing space hydration always inserts).
  const d = doc(para(t("@Ada ")));
  for (const pos of [2, 3, 4]) {
    assert.equal(
      snapCaretOutOfAgentMention(d, pos, ["Ada"]),
      6,
      `interior pos ${pos} should snap after trailing space`,
    );
  }
});

test("snapCaretOutOfAgentMention snaps to name end when no trailing space", () => {
  const d = doc(para(t("@Ada")));
  assert.equal(snapCaretOutOfAgentMention(d, 3, ["Ada"]), 5);
});

test("snapCaretOutOfAgentMention ignores non-agent mention names", () => {
  // Human-only name list: no snap (agents are the ones with hidden @ + auto-tag).
  const d = doc(para(t("@Alice hello")));
  assert.equal(snapCaretOutOfAgentMention(d, 3, []), 3);
});

test("snapCaretOutOfAgentMention handles multiple agent chips", () => {
  // "@Ada @Bob " — caret inside Bob should land after Bob's trailing space.
  const d = doc(para(t("@Ada @Bob more")));
  const ranges = findAgentMentionRanges(d, ["Ada", "Bob"]);
  assert.equal(ranges.length, 2);
  const bob = ranges[1];
  const interior = bob.from + 2;
  const snapped = snapCaretOutOfAgentMention(d, interior, ["Ada", "Bob"]);
  assert.equal(snapped, bob.to + 1); // consume trailing space after @Bob
});

// ── Atomic Backspace/Delete on agent chips (#2707) ────────────────────

test("withTrailingSpace consumes a single trailing space", () => {
  const d = doc(para(t("@Ada more")));
  // "@Ada" is 1..5; trailing space at 5 → 6
  assert.equal(withTrailingSpace(d, 5), 6);
  assert.equal(withTrailingSpace(d, 6), 6); // already past space
});

test("agentMentionChipDeleteRange: interior Backspace/Delete remove whole chip", () => {
  const d = doc(para(t("@Ada hello")));
  // "@Ada" 1..5, chip with space 1..6
  for (const key of /** @type {const} */ (["Backspace", "Delete"])) {
    for (const pos of [2, 3, 4]) {
      assert.deepEqual(
        agentMentionChipDeleteRange(d, pos, ["Ada"], key),
        { from: 1, to: 6 },
        `${key} at interior ${pos}`,
      );
    }
  }
});

test("agentMentionChipDeleteRange: Backspace at end of name or after space", () => {
  const d = doc(para(t("@Ada hello")));
  // end of name (5) and after trailing space (6)
  assert.deepEqual(agentMentionChipDeleteRange(d, 5, ["Ada"], "Backspace"), {
    from: 1,
    to: 6,
  });
  assert.deepEqual(agentMentionChipDeleteRange(d, 6, ["Ada"], "Backspace"), {
    from: 1,
    to: 6,
  });
  // Deep in following text: default editing
  assert.equal(agentMentionChipDeleteRange(d, 10, ["Ada"], "Backspace"), null);
});

test("agentMentionChipDeleteRange: Delete at start of chip only", () => {
  const d = doc(para(t("@Ada hello")));
  assert.deepEqual(agentMentionChipDeleteRange(d, 1, ["Ada"], "Delete"), {
    from: 1,
    to: 6,
  });
  // Delete from after the chip leaves following text alone
  assert.equal(agentMentionChipDeleteRange(d, 6, ["Ada"], "Delete"), null);
});

test("agentMentionChipDeleteRange: Backspace between two agent chips removes the left one", () => {
  // "@Ada @Bob" — caret after Ada's trailing space is Bob's `@`
  const d = doc(para(t("@Ada @Bob")));
  const ranges = findAgentMentionRanges(d, ["Ada", "Bob"]);
  const ada = ranges[0];
  const bob = ranges[1];
  const afterAdaSpace = withTrailingSpace(d, ada.to);
  assert.equal(afterAdaSpace, bob.from);
  assert.deepEqual(
    agentMentionChipDeleteRange(d, afterAdaSpace, ["Ada", "Bob"], "Backspace"),
    { from: ada.from, to: afterAdaSpace },
  );
  // Delete at that same spot removes Bob (start of next chip)
  assert.deepEqual(
    agentMentionChipDeleteRange(d, afterAdaSpace, ["Ada", "Bob"], "Delete"),
    { from: bob.from, to: withTrailingSpace(d, bob.to) },
  );
});

test("agentMentionChipDeleteRange: chip without trailing space", () => {
  const d = doc(para(t("@Ada")));
  assert.deepEqual(agentMentionChipDeleteRange(d, 3, ["Ada"], "Backspace"), {
    from: 1,
    to: 5,
  });
  assert.deepEqual(agentMentionChipDeleteRange(d, 5, ["Ada"], "Backspace"), {
    from: 1,
    to: 5,
  });
});
