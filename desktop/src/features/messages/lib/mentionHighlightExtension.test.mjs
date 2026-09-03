import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState, TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import {
  assignMentionHighlightNames,
  buildHighlightPatterns,
  createMentionCaretSettlement,
  findHighlightMatches,
  insertionForMentionTextInput,
  MentionHighlightExtension,
  mentionHighlightKey,
  mentionTextInputInsertion,
  positionAfterArrowLeftThroughMentionSpace,
  selectionAfterMentionTrailingSpace,
  shouldAdvanceMentionCaret,
  settleAutocompleteMentionInsert,
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
    }),
    false,
  );
});

test("shouldAdvanceMentionCaret does not advance on an unsettled document change", () => {
  // Regression: `docChanged` used to force an advance. That walked the caret
  // across a mention's trailing space on every keystroke, so typing `@name`
  // before existing text interleaved spaces into the draft.
  assert.equal(
    shouldAdvanceMentionCaret({ from: 7, next: 8, settling: false }),
    false,
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

test("insertionForMentionTextInput redirects a caret at the chip edge", () => {
  const doc = document(paragraph(text("@quinn ")));
  const spacePos = 1 + "@quinn".length;
  assert.deepEqual(insertionForMentionTextInput(doc, spacePos, spacePos, "x"), {
    insertAt: spacePos + 1,
    text: "x",
  });
  assert.equal(
    insertionForMentionTextInput(doc, spacePos + 1, spacePos + 1, "x"),
    null,
  );
});

test("insertionForMentionTextInput keeps a selected trailing space", () => {
  const doc = document(paragraph(text("@quinn ")));
  const spacePos = 1 + "@quinn".length;
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos, spacePos + 1, "x"),
    { insertAt: spacePos + 1, text: "x" },
  );
});

test("insertionForMentionTextInput keeps the draft space in a whitespace-run rewrite", () => {
  // Chromium can rewrite the whole space run when typing between the
  // mention's trailing space and a pre-existing draft space, emitting
  // replace("  " -> " a") — usually with a non-breaking space, and anchored
  // at either edge of the run. The draft's space must survive every shape.
  const doc = document(paragraph(text("hello @bob  world")));
  const spacePos = 1 + "hello @bob".length;
  const kept = { insertAt: spacePos + 1, text: "a" };
  // Anchored at the chip edge, replacing the whole run.
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos, spacePos + 2, " a"),
    kept,
  );
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos, spacePos + 2, "\u00A0a"),
    kept,
  );
  // Anchored past the trailing space, rewriting only the draft's own space.
  // This shape used to fall through to the destructive default and produce
  // "hello @bob abcworld" — the failure CI caught.
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos + 1, spacePos + 2, "\u00A0a"),
    kept,
  );
  // Trailing whitespace is the run being re-emitted on the other side.
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos, spacePos + 2, "a\u00A0"),
    kept,
  );
  // Whatever the shape, replaced whitespace is never dropped while settling.
  assert.deepEqual(
    insertionForMentionTextInput(doc, spacePos, spacePos + 2, "x"),
    { insertAt: spacePos + 1, text: "x" },
  );
  // Replacing something other than whitespace is a real edit — leave it.
  assert.equal(
    insertionForMentionTextInput(doc, spacePos, spacePos + 3, " a"),
    null,
  );
});

test("mentionTextInputInsertion honors a deliberate caret after settlement", () => {
  const doc = document(paragraph(text("@bob ")));
  const spacePos = 1 + "@bob".length;
  assert.equal(
    mentionTextInputInsertion(doc, spacePos, spacePos, "x", false),
    null,
  );
  assert.deepEqual(
    mentionTextInputInsertion(doc, spacePos, spacePos, "x", true),
    { insertAt: spacePos + 1, text: "x" },
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

// ── caret regression: typing a mention before existing text ───────────
//
// Drives the real plugin through EditorState so `appendTransaction` runs.
// Before the fix, every keystroke advanced the caret across a mention's
// trailing space, interleaving spaces into the rest of the draft.

function editorStateWithMentionHighlight(initialText, names) {
  // The plugin factory only reads `this.storage`, so a minimal stand-in is
  // enough to exercise it without constructing a full editor (needs a DOM).
  const plugins = MentionHighlightExtension.config.addProseMirrorPlugins.call({
    storage: { names, agentNames: [], channelNames: [] },
  });
  return EditorState.create({
    doc: document(paragraph(text(initialText))),
    schema,
    plugins,
  });
}

/** Insert `typed` one character at a time at `startPos`, as typing does. */
function typeAt(state, startPos, typed) {
  let next = state.apply(
    state.tr.setSelection(TextSelection.create(state.doc, startPos)),
  );
  for (const char of typed) {
    const { from, to } = next.selection;
    const tr = next.tr.insertText(char, from, to);
    tr.setSelection(TextSelection.create(tr.doc, tr.mapping.map(to, 1)));
    next = next.apply(tr);
  }
  return next;
}

test("typing a mention before existing text does not interleave spaces", () => {
  // Caret placed just after "hello" in "hello world", then " @qu" is typed.
  const state = editorStateWithMentionHighlight("hello world", ["quinn"]);
  const typed = typeAt(state, 1 + "hello".length, " @qu");
  assert.equal(typed.doc.textContent, "hello @qu world");
});

test("typing a mention before existing text keeps the caret with the text", () => {
  const state = editorStateWithMentionHighlight("hello world", ["quinn"]);
  const typed = typeAt(state, 1 + "hello".length, " @qu");
  assert.equal(typed.selection.from, 1 + "hello @qu".length);
});

test("an unknown @token before existing text is not rewritten either", () => {
  // The trailing-space scan is purely textual, so it fired even for names
  // that were never registered as mentions.
  const state = editorStateWithMentionHighlight("hello world", []);
  const typed = typeAt(state, 1 + "hello".length, " @zz");
  assert.equal(typed.doc.textContent, "hello @zz world");
});

test("typing a mention at the end of a message still works", () => {
  const state = editorStateWithMentionHighlight("hello world", ["quinn"]);
  const typed = typeAt(state, 1 + "hello world".length, " @qu");
  assert.equal(typed.doc.textContent, "hello world @qu");
});

test("typing after a completed mention keeps the separator intact", () => {
  // "@quinn " already exists; typing at the very end must not consume or
  // duplicate the trailing space.
  const state = editorStateWithMentionHighlight("@quinn world", ["quinn"]);
  const typed = typeAt(state, 1 + "@quinn world".length, "!");
  assert.equal(typed.doc.textContent, "@quinn world!");
});

// ── the browser branch: Chromium's whitespace-run rewrite ─────────────
//
// The helper tests above model the payload. These drive the plugin's real
// `handleTextInput` prop and fall back to ProseMirror's default insertion
// when it declines, exactly as the browser does with the return value — so
// the assertion is sensitive to the production branch itself rather than to
// winning a timing race in a headless browser.

/** Apply an autocomplete pick: replace the typed token and settle the caret. */
function pickMentionAt(state, tokenFrom, tokenTo, inserted) {
  const tr = state.tr.insertText(inserted, tokenFrom, tokenTo);
  tr.setSelection(TextSelection.create(tr.doc, tokenFrom + inserted.length));
  tr.setMeta(mentionHighlightKey, true);
  return state.apply(tr);
}

/** Route a text-input event through the plugin, then the default handling. */
function textInput(state, from, to, text) {
  let current = state;
  const view = {
    get state() {
      return current;
    },
    dispatch(tr) {
      current = current.apply(tr);
    },
    domAtPos: () => ({ node: {}, offset: 0 }),
    root: undefined,
  };
  const handled = current.plugins.some(
    (plugin) => plugin.props?.handleTextInput?.(view, from, to, text) === true,
  );
  return handled
    ? current
    : current.apply(current.tr.insertText(text, from, to));
}

test("a whitespace-run rewrite after a mention pick keeps the draft space", () => {
  // The CI failure: with "hello world" drafted, the caret placed after
  // "hello", " @bo" typed and the "bob" suggestion picked, the document is
  // "hello @bob  world" — the mention's trailing space followed by the
  // draft's own space. The next keystroke arrives as one of these shapes
  // depending on how Chromium reconciles that whitespace run, and all of
  // them have to keep both spaces.
  const tokenFrom = 1 + "hello ".length;
  const spacePos = 1 + "hello @bob".length;
  const shapes = [
    { name: "caret at the chip edge", from: spacePos, to: spacePos, text: "a" },
    {
      name: "caret past the trailing space",
      from: spacePos + 1,
      to: spacePos + 1,
      text: "a",
    },
    {
      name: "run rewritten from the chip edge",
      from: spacePos,
      to: spacePos + 2,
      text: "\u00A0a",
    },
    {
      name: "draft space rewritten on its own",
      from: spacePos + 1,
      to: spacePos + 2,
      text: "\u00A0a",
    },
  ];

  for (const shape of shapes) {
    // A fresh editor per shape: settlement is per-plugin closure state.
    const picked = pickMentionAt(
      editorStateWithMentionHighlight("hello @bo world", ["bob"]),
      tokenFrom,
      tokenFrom + "@bo".length,
      "@bob ",
    );
    assert.equal(picked.doc.textContent, "hello @bob  world", shape.name);
    const typed = textInput(picked, shape.from, shape.to, shape.text);
    assert.equal(typed.doc.textContent, "hello @bob a world", shape.name);
  }
});

// Multi-word display names must use the same settlement as single-word names.
// Simulate the browser remapping the DOM caret to the chip edge before typing.
test("multi-word autocomplete keeps its separator after a chip-edge remap", () => {
  const storage = { names: ["Remote Scout"], agentNames: [], channelNames: [] };
  const plugins = MentionHighlightExtension.config.addProseMirrorPlugins.call({
    storage,
  });
  const state = EditorState.create({
    doc: document(paragraph(text("@Remote"))),
    schema,
    plugins,
  });
  const tr = state.tr.insertText("@Remote Scout ", 1, 8);
  tr.setSelection(TextSelection.create(tr.doc, 15));
  settleAutocompleteMentionInsert(
    { storage: { mentionHighlight: storage } },
    tr,
    "@Remote Scout ",
  );
  const picked = state.apply(tr);
  const spacePos = 1 + "@Remote Scout".length;
  const typed = textInput(picked, spacePos, spacePos, "hello");
  assert.equal(typed.doc.textContent, "@Remote Scout hello");
});

test("full-name boundaries preserve internal spaces and intentional caret moves", () => {
  const names = ["Remote Scout", "Scout (ed12)"];
  const doc = document(paragraph(text("@Remote Scout  world")));
  const edge = 1 + "@Remote Scout".length;
  assert.equal(
    selectionAfterMentionTrailingSpace(doc, 1 + "@Remote".length, names),
    1 + "@Remote".length,
  );
  assert.equal(selectionAfterMentionTrailingSpace(doc, edge, names), edge + 1);
  assert.equal(
    positionAfterArrowLeftThroughMentionSpace(doc, edge + 1, names),
    edge,
  );
  assert.equal(
    mentionTextInputInsertion(doc, edge, edge, "x", false, names),
    null,
  );
  assert.deepEqual(
    insertionForMentionTextInput(doc, edge, edge + 2, "\u00a0x", names),
    { insertAt: edge + 1, text: "x" },
  );
  const disambiguated = document(paragraph(text("@Scout (ed12) ")));
  assert.equal(
    selectionAfterMentionTrailingSpace(
      disambiguated,
      1 + "@Scout (ed12)".length,
      names,
    ),
    1 + "@Scout (ed12) ".length,
  );
});
