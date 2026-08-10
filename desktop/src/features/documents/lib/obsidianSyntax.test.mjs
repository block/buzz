import assert from "node:assert/strict";
import { test } from "node:test";

import {
  activeHeadingIndex,
  findBlockId,
  findComments,
  findHighlights,
  findTags,
  parseCallout,
} from "./obsidianSyntax.ts";

const raws = (matches) => matches.map((m) => m.raw);
const contents = (matches) => matches.map((m) => m.content);

test("finds highlights", () => {
  assert.deepEqual(contents(findHighlights("a ==marked== b")), ["marked"]);
  assert.deepEqual(contents(findHighlights("==one== and ==two==")), [
    "one",
    "two",
  ]);
});

test("ignores highlight lookalikes", () => {
  // A setext underline and an empty pair are not highlights.
  assert.deepEqual(findHighlights("===="), []);
  assert.deepEqual(findHighlights("== =="), []);
  assert.deepEqual(findHighlights("a = b == c"), []);
});

test("finds comments", () => {
  assert.deepEqual(contents(findComments("text %%hidden%% more")), ["hidden"]);
  assert.deepEqual(findComments("100%% done"), []);
});

test("finds tags including nested ones", () => {
  assert.deepEqual(raws(findTags("a #work and #work/buzz")), [
    "#work",
    "#work/buzz",
  ]);
  assert.deepEqual(raws(findTags("#start-of-line")), ["#start-of-line"]);
});

test("does not mistake headings, colours or fragments for tags", () => {
  // A markdown heading has a space after the hashes.
  assert.deepEqual(findTags("# Heading"), []);
  assert.deepEqual(findTags("### Sub heading"), []);
  // A bare hash, and a hex colour, are not tags.
  assert.deepEqual(findTags("just # alone"), []);
  assert.deepEqual(findTags("colour #123"), []);
  // Mid-word hashes (including URL fragments) are left alone.
  assert.deepEqual(findTags("see https://x.com/a#frag"), []);
  assert.deepEqual(findTags("foo#bar"), []);
});

test("parses callouts and their aliases", () => {
  assert.deepEqual(parseCallout("> [!info] Heads up"), {
    canonical: "info",
    title: "Heads up",
    type: "info",
  });
  // Aliases collapse onto a canonical style.
  assert.equal(parseCallout("> [!tldr]").canonical, "summary");
  assert.equal(parseCallout("> [!caution]").canonical, "warning");
  assert.equal(parseCallout("> [!help]").canonical, "question");
  // Case-insensitive, and the fold marker is tolerated.
  assert.equal(parseCallout("> [!WARNING]-").canonical, "warning");
  assert.equal(parseCallout("> [!note]+ Title").title, "Title");
});

test("an untitled callout has a null title", () => {
  assert.equal(parseCallout("> [!info]").title, null);
  assert.equal(parseCallout("> [!info]   ").title, null);
});

test("an unknown callout type still renders as a callout", () => {
  // Obsidian falls back to default styling rather than dropping to a quote.
  assert.deepEqual(parseCallout("> [!bogus] x"), {
    canonical: "note",
    title: "x",
    type: "bogus",
  });
});

test("a plain blockquote is not a callout", () => {
  assert.equal(parseCallout("> just a quote"), null);
  assert.equal(parseCallout("not a quote at all"), null);
  assert.equal(parseCallout("> [not a callout]"), null);
});

test("finds a trailing block-id anchor", () => {
  assert.equal(findBlockId("A claim. ^my-block").content, "my-block");
  assert.equal(findBlockId("Trailing space. ^abc123  ").content, "abc123");
});

test("a caret inside a wikilink is not a block anchor", () => {
  // `[[Note^id]]` is a block *reference*; the anchor form only ends a line.
  assert.equal(findBlockId("See [[Note^id]] here."), null);
  assert.equal(findBlockId("See [[Note^id]]"), null);
});

test("ignores carets that are not anchors", () => {
  assert.equal(findBlockId("2^10 is 1024"), null);
  assert.equal(findBlockId("no caret at all"), null);
  assert.equal(findBlockId("^leading-only"), null, "needs preceding space");
});

test("scroll-spy picks the last heading at or above the viewport", () => {
  const offsets = [0, 100, 250];
  assert.equal(activeHeadingIndex(offsets, 0), 0);
  assert.equal(activeHeadingIndex(offsets, 50), 0);
  assert.equal(activeHeadingIndex(offsets, 100), 1);
  assert.equal(activeHeadingIndex(offsets, 200), 1);
  assert.equal(activeHeadingIndex(offsets, 1000), 2);
});

test("scroll-spy activates a heading just before it reaches the top", () => {
  // The 8px tolerance stops the active item flickering when a heading sits
  // exactly on the viewport edge, so it engages slightly early by design.
  const offsets = [0, 100];
  assert.equal(activeHeadingIndex(offsets, 91), 0, "outside the tolerance");
  assert.equal(activeHeadingIndex(offsets, 92), 1, "inside the tolerance");
});

test("scroll-spy reports nothing active above the first heading", () => {
  assert.equal(activeHeadingIndex([200, 400], 0), -1);
  assert.equal(activeHeadingIndex([], 0), -1);
});
