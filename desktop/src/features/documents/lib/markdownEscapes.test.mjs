import assert from "node:assert/strict";
import { test } from "node:test";

import {
  normalizeHardBreaks,
  stripMarkdownEscapes,
  toDiskMarkdown,
} from "./markdownEscapes.ts";

test("unescapes the characters prosemirror-markdown escapes", () => {
  // The case that motivates the whole function: a wikilink must survive.
  assert.equal(
    stripMarkdownEscapes("A \\[\\[wikilink\\]\\] here."),
    "A [[wikilink]] here.",
  );
  assert.equal(stripMarkdownEscapes("\\*not bold\\*"), "*not bold*");
  assert.equal(stripMarkdownEscapes("\\`code\\`"), "`code`");
  assert.equal(stripMarkdownEscapes("\\~\\~strike\\~\\~"), "~~strike~~");
  assert.equal(stripMarkdownEscapes("snake\\_case"), "snake_case");
  assert.equal(stripMarkdownEscapes("\\!\\[alt\\]"), "![alt]");
});

test("leaves unescaped text alone", () => {
  const plain = "Nothing to undo here — [[link]] *bold* `code`.";
  assert.equal(stripMarkdownEscapes(plain), plain);
});

test("strips exactly one level of escaping", () => {
  // `\\[` is an escaped backslash followed by a bracket. Removing the first
  // backslash is correct; the bracket must not also lose its escape.
  assert.equal(stripMarkdownEscapes("\\\\["), "\\[");
});

test("known false positive: a deliberate literal escape is lost", () => {
  // Documented, not accidental. A user who typed `\*` to mean a literal
  // asterisk gets `*` back, which will render as emphasis next time. The
  // round-trip guard is what actually protects such files -- they classify
  // lossy and open in source mode, never reaching this function.
  assert.equal(stripMarkdownEscapes("2 \\* 3"), "2 * 3");
});

test("collapses CommonMark hard line breaks", () => {
  assert.equal(
    normalizeHardBreaks("line one\\\nline two"),
    "line one\nline two",
  );
});

test("toDiskMarkdown applies both passes", () => {
  assert.equal(
    toDiskMarkdown("A \\[\\[link\\]\\]\\\nnext line"),
    "A [[link]]\nnext line",
  );
});
