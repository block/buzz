import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

import { buildPlainTextProjection } from "./plainTextProjection.ts";

// ── Build the actual Tiptap schema we use in the composer ─────────────
// Matching useRichTextEditor's StarterKit configuration (minus things
// that don't affect the schema shape). This guarantees the projection
// is tested against the *real* node names and types.

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
const br = () => schema.nodes.hardBreak.create();
const li = (...c) => schema.nodes.listItem.create(null, c);
const ul = (...c) => schema.nodes.bulletList.create(null, c);

function doc(...content) {
  return schema.nodes.doc.create(null, content);
}

// ── Helper: assert text equals textBetween(blockSep, leafText="\n") ───

function assertMatchesTextBetween(d, p) {
  const expected = d.textBetween(0, d.content.size, "\n", "\n");
  assert.equal(
    p.text,
    expected,
    `projection.text should equal doc.textBetween(0..size, "\\n", "\\n")`,
  );
}

// ── Single-paragraph ──────────────────────────────────────────────────

test("single paragraph: text is the paragraph's content", () => {
  const d = doc(para(t("hello")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "hello");
  assertMatchesTextBetween(d, p);
});

test("single paragraph: PM↔text mapping is identity within text node", () => {
  const d = doc(para(t("hello")));
  const p = buildPlainTextProjection(d);
  // PM: para=0, "hello"=1..6
  assert.equal(p.mapPMToTextOffset(1), 0);
  assert.equal(p.mapPMToTextOffset(3), 2);
  assert.equal(p.mapPMToTextOffset(6), 5);
  assert.equal(p.mapTextOffsetToPM(0), 1);
  assert.equal(p.mapTextOffsetToPM(2), 3);
  assert.equal(p.mapTextOffsetToPM(5), 6);
});

// ── HardBreak ─────────────────────────────────────────────────────────

test("hardBreak contributes a single \\n", () => {
  const d = doc(para(t("hello"), br(), t("world")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "hello\nworld");
  assertMatchesTextBetween(d, p);
});

test("cursor before <br> maps to text offset just before the \\n", () => {
  // PM: para=0, "hello"=1..5, <br>=6, "world"=7..11
  const d = doc(para(t("hello"), br(), t("world")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapPMToTextOffset(6), 5);
});

test("cursor after <br> maps to text offset just after the \\n", () => {
  const d = doc(para(t("hello"), br(), t("world")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapPMToTextOffset(7), 6);
});

test("text offset right after \\n maps to PM after the break", () => {
  const d = doc(para(t("hello"), br(), t("world")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(6), 7);
});

test("text offset just before \\n maps to PM before the break", () => {
  const d = doc(para(t("hello"), br(), t("world")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(5), 6);
});

// ── Multi-paragraph ───────────────────────────────────────────────────

test("two paragraphs: block boundary contributes a single \\n", () => {
  const d = doc(para(t("aaa")), para(t("bbb")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "aaa\nbbb");
  assertMatchesTextBetween(d, p);
});

test("two paragraphs: cursor in second paragraph maps past the boundary \\n", () => {
  // PM: p1 nodeSize=5 (token + 3 chars + token), p2 opens at PM=5
  //     "aaa" text at 1..4 (size 3), p1 closes at PM=4..5, p2 opens at PM=5,
  //     "bbb" text at 6..9
  const d = doc(para(t("aaa")), para(t("bbb")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapPMToTextOffset(6), 4);
  assert.equal(p.mapTextOffsetToPM(4), 6);
});

test("three paragraphs: cumulative block boundaries", () => {
  const d = doc(para(t("aaa")), para(t("bbb")), para(t("ccc")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "aaa\nbbb\nccc");
  assertMatchesTextBetween(d, p);
  // "ccc" text starts at PM=11 → text offset 8
  assert.equal(p.mapPMToTextOffset(11), 8);
  assert.equal(p.mapTextOffsetToPM(8), 11);
});

// ── HardBreak + multi-paragraph ──────────────────────────────────────

test("paragraph with <br> then second paragraph", () => {
  const d = doc(para(t("line1"), br(), t("line2")), para(t("para2")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "line1\nline2\npara2");
  assertMatchesTextBetween(d, p);
});

test("range crossing a <br>", () => {
  // PM: para=0, "line1"=1..5, <br>=6, "line2"=7..11
  // textOffset 2..8 = "ne1\nli" → PM 3..9
  const d = doc(para(t("line1"), br(), t("line2")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(2), 3);
  assert.equal(p.mapTextOffsetToPM(8), 9);
});

// ── List items (nested blocks under bulletList) ──────────────────────

test("bullet list: items separated by \\n (matches textBetween)", () => {
  const d = doc(ul(li(para(t("first"))), li(para(t("second")))));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "first\nsecond");
  assertMatchesTextBetween(d, p);
});

test("paragraph + bullet list", () => {
  const d = doc(para(t("intro")), ul(li(para(t("a"))), li(para(t("b")))));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "intro\na\nb");
  assertMatchesTextBetween(d, p);
});

test("list item: PM↔text round-trip lands in the right item", () => {
  const d = doc(ul(li(para(t("first"))), li(para(t("second")))));
  const p = buildPlainTextProjection(d);
  const pm = p.mapTextOffsetToPM(6); // start of "second"
  assert.equal(p.mapPMToTextOffset(pm), 6);
});

// ── Edge cases ───────────────────────────────────────────────────────

test("offset 0 maps inside the first text node", () => {
  const d = doc(para(t("hello")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(0), 1);
});

test("offset past end clamps to end-of-doc content position", () => {
  const d = doc(para(t("hello")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(999), 6);
});

test("PM position past doc clamps to text.length", () => {
  const d = doc(para(t("hello")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapPMToTextOffset(999), 5);
});

test("empty paragraph: empty text, mappings clamp safely", () => {
  const d = doc(para());
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "");
  assert.equal(p.mapPMToTextOffset(0), 0);
  assert.equal(p.mapPMToTextOffset(1), 0);
  // Inside the empty paragraph → PM=1
  assert.equal(p.mapTextOffsetToPM(0), 1);
});

// ── Empty leaf blocks (paste / draft restore can produce these) ─────

test("empty paragraph after text: trailing \\n preserved", () => {
  const d = doc(para(t("a")), para());
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "a\n");
  assertMatchesTextBetween(d, p);
});

test("empty paragraph after text: offset round-trips into the empty block", () => {
  // PM: p1=0..2 (size 3: <p>1 + "a"1 + </p>1), text "a" at 1..2
  //     p2=3..4 (size 2: <p>1 + </p>1), empty interior at PM=4
  const d = doc(para(t("a")), para());
  const p = buildPlainTextProjection(d);
  // offset 2 (right after the `\n`) → PM=4 (inside empty p2) → back to 2
  assert.equal(p.mapTextOffsetToPM(2), 4);
  assert.equal(p.mapPMToTextOffset(4), 2);
});

test("empty paragraph before text: leading \\n preserved", () => {
  const d = doc(para(), para(t("a")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "\na");
  assertMatchesTextBetween(d, p);
});

test("empty paragraph before text: offset 0 lands inside the empty block", () => {
  // PM: p1=0..1 (size 2: <p>1 + </p>1), empty interior at PM=1
  //     p2=2..4 (size 3), "a" at PM=3
  const d = doc(para(), para(t("a")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.mapTextOffsetToPM(0), 1);
  assert.equal(p.mapPMToTextOffset(1), 0);
});

test("two empty paragraphs: single \\n separator, both interiors reachable", () => {
  const d = doc(para(), para());
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "\n");
  assertMatchesTextBetween(d, p);
  // PM: p1=0..1 (size 2), interior=1. p2=2..3 (size 2), interior=3.
  assert.equal(p.mapTextOffsetToPM(0), 1);
  assert.equal(p.mapTextOffsetToPM(1), 3);
  assert.equal(p.mapPMToTextOffset(1), 0);
  assert.equal(p.mapPMToTextOffset(3), 1);
});

test("empty list item: interior reachable, separators preserved", () => {
  const d = doc(ul(li(para()), li(para(t("x")))));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "\nx");
  assertMatchesTextBetween(d, p);
});

// ── Property: round-trip ─────────────────────────────────────────────

test("round-trip: text offset → PM → text offset is identity", () => {
  const d = doc(
    para(t("line1"), br(), t("line2")),
    para(),
    para(t("para3")),
    ul(li(para(t("item-a"))), li(para()), li(para(t("item-c")))),
  );
  const p = buildPlainTextProjection(d);
  for (let offset = 0; offset <= p.text.length; offset++) {
    const pm = p.mapTextOffsetToPM(offset);
    const back = p.mapPMToTextOffset(pm);
    assert.equal(
      back,
      offset,
      `offset ${offset} → pm ${pm} → offset ${back} (text=${JSON.stringify(p.text)})`,
    );
  }
});

// ── Custom-emoji atom node ───────────────────────────────────────────
// The node is an inline leaf that is 1 PM position wide but projects to
// its full `:shortcode:` text. The projection must account for that width
// mismatch so cursor math and autocomplete offsets stay correct.

import { CustomEmojiNode } from "./customEmojiNode.ts";

const schemaWithEmoji = getSchema([
  StarterKit.configure({
    hardBreak: { keepMarks: true },
    heading: false,
    trailingNode: false,
    link: false,
  }),
  CustomEmojiNode,
]);

const eDoc = (...content) => schemaWithEmoji.nodes.doc.create(null, content);
const ePara = (...c) => schemaWithEmoji.nodes.paragraph.create(null, c);
const eText = (s) => schemaWithEmoji.text(s);
const emoji = (shortcode) =>
  schemaWithEmoji.nodes.customEmoji.create({ shortcode, src: "" });

test("atom: projects to its full :shortcode: text", () => {
  const d = eDoc(ePara(eText("hi "), emoji("wave"), eText(" there")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "hi :wave: there");
});

test("atom: text matches textBetween (renderText-equivalent width)", () => {
  const d = eDoc(ePara(eText("a"), emoji("party_parrot"), eText("b")));
  const p = buildPlainTextProjection(d);
  // textBetween uses the node's `leafText` (renderText) → `:shortcode:`.
  assert.equal(p.text, "a:party_parrot:b");
});

test("atom: PM before/after maps to text edges", () => {
  // doc(para(text("hi "), emoji, text(" x")))
  //   pos 0 = doc start; para opens at 0, content begins at 1.
  //   "hi " is 3 chars → PM 1..4; emoji atom at PM 4 (1 wide) → 4..5;
  //   " x" → PM 5..7.
  const d = eDoc(ePara(eText("hi "), emoji("wave"), eText(" x")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "hi :wave: x");
  // PM 4 (just before the atom) → text offset 3 (end of "hi ").
  assert.equal(p.mapPMToTextOffset(4), 3);
  // PM 5 (just after the atom) → text offset 9 (after ":wave:").
  assert.equal(p.mapPMToTextOffset(5), "hi :wave:".length);
});

test("atom: text offsets inside the shortcode snap to the node's left edge", () => {
  const d = eDoc(ePara(emoji("wave"), eText("x")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, ":wave:x");
  const leftPM = p.mapTextOffsetToPM(0); // before the atom
  // Any offset within ":wave:" (0..5) cannot land inside the atom → left edge.
  for (let off = 0; off < ":wave:".length; off++) {
    assert.equal(
      p.mapTextOffsetToPM(off),
      leftPM,
      `offset ${off} inside shortcode should snap to the node's left edge`,
    );
  }
  // The offset at the right edge of ":wave:" lands after the atom (the "x").
  const afterPM = p.mapTextOffsetToPM(":wave:".length);
  assert.ok(afterPM > leftPM, "right edge should be past the atom");
});

test("atom: caret offsets around an atom round-trip", () => {
  // Only the *caret* positions (before/after the atom, not inside the
  // shortcode interior) are expected to round-trip.
  const d = eDoc(ePara(eText("ab"), emoji("wave"), eText("cd")));
  const p = buildPlainTextProjection(d);
  assert.equal(p.text, "ab:wave:cd");
  const caretTextOffsets = [
    0,
    1,
    2, // before the atom (end of "ab")
    "ab:wave:".length, // after the atom (start of "cd")
    "ab:wave:c".length,
    "ab:wave:cd".length,
  ];
  for (const offset of caretTextOffsets) {
    const pm = p.mapTextOffsetToPM(offset);
    const back = p.mapPMToTextOffset(pm);
    assert.equal(back, offset, `caret offset ${offset} → pm ${pm} → ${back}`);
  }
});
