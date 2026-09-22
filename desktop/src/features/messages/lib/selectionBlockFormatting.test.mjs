import assert from "node:assert/strict";
import test from "node:test";

import { getSchema, Node } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { EditorState, TextSelection } from "@tiptap/pm/state";

import { CustomEmojiNode } from "./customEmojiNode.ts";

import {
  isolateSelectionForBlockFormatting,
  mergeSelectedTextblocksIntoCodeBlock,
  splitSelectedLinesForListFormatting,
} from "./selectionBlockFormatting.ts";

// Matching useRichTextEditor's StarterKit configuration (minus things
// irrelevant to block isolation).
const MentionNode = Node.create({
  name: "mention",
  group: "inline",
  inline: true,
  atom: true,
  addAttributes: () => ({ label: { default: "" } }),
});
const UnknownLeaf = Node.create({
  name: "unknownLeaf",
  group: "inline",
  inline: true,
  atom: true,
  addAttributes: () => ({ internalId: { default: "secret" } }),
});

const schema = getSchema([
  StarterKit.configure({
    hardBreak: { keepMarks: true },
    heading: false,
    trailingNode: false,
    link: false,
  }),
  MentionNode,
  CustomEmojiNode.configure({
    resolveUrl: () => undefined,
    shortcodes: () => [],
  }),
  UnknownLeaf,
]);

const para = (...content) => schema.nodes.paragraph.create(null, content);
const br = () => schema.nodes.hardBreak.create();
const t = (text) => schema.text(text);

function doc(...content) {
  return schema.nodes.doc.create(null, content);
}

function stateWithCaret(documentNode, caret) {
  return EditorState.create({
    doc: documentNode,
    selection: TextSelection.create(documentNode, caret),
  });
}

function paragraphTexts(documentNode) {
  const texts = [];
  documentNode.forEach((node) => {
    texts.push(node.textContent);
  });
  return texts;
}

test("caret between hard breaks isolates only its line", () => {
  // <p>before␍target␍after</p> with the caret inside "target".
  const state = stateWithCaret(
    doc(para(t("before"), br(), t("target"), br(), t("after"))),
    10,
  );

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), true);

  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["before", "target", "after"]);
  assert.equal(next.selection.empty, true);
  assert.equal(next.selection.$from.parent.textContent, "target");
});

test("caret on an empty trailing line isolates an empty paragraph", () => {
  // "before" + Shift+Enter, caret at the end — the reported bug shape.
  const state = stateWithCaret(doc(para(t("before"), br())), 8);

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), true);

  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["before", ""]);
  assert.equal(next.selection.empty, true);
  assert.equal(next.selection.$from.parent.textContent, "");
});

test("caret on the first line splits only after that line", () => {
  const state = stateWithCaret(doc(para(t("first"), br(), t("rest"))), 3);

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), true);

  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["first", "rest"]);
  assert.equal(next.selection.$from.parent.textContent, "first");
});

test("caret on the last line splits only before that line", () => {
  const state = stateWithCaret(doc(para(t("rest"), br(), t("last"))), 8);

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), true);

  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["rest", "last"]);
  assert.equal(next.selection.$from.parent.textContent, "last");
});

test("caret in a single-line paragraph is a no-op", () => {
  const state = stateWithCaret(doc(para(t("only line"))), 4);

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), false);
  assert.equal(transaction.steps.length, 0);
});

test("exact block-boundary selection excludes endpoint paragraphs", () => {
  const documentNode = doc(para(t("alpha")), para(t("beta")), para(t("gamma")));
  for (const backward of [false, true]) {
    const state = EditorState.create({
      doc: documentNode,
      selection: TextSelection.create(
        documentNode,
        backward ? 14 : 6,
        backward ? 6 : 14,
      ),
    });
    const transaction = state.tr;
    isolateSelectionForBlockFormatting(transaction);
    assert.equal(mergeSelectedTextblocksIntoCodeBlock(transaction), true);
    const next = state.apply(transaction);
    assert.deepEqual(
      next.doc.toJSON(),
      doc(
        para(t("alpha")),
        schema.nodes.codeBlock.create(null, t("beta")),
        para(t("gamma")),
      ).toJSON(),
    );
  }
});

test("selection isolation still splits around the selected text", () => {
  const documentNode = doc(para(t("before selected after")));
  const state = EditorState.create({
    doc: documentNode,
    // "selected" spans positions 8..16.
    selection: TextSelection.create(documentNode, 8, 16),
  });

  const transaction = state.tr;
  assert.equal(isolateSelectionForBlockFormatting(transaction), true);

  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["before ", "selected", " after"]);
  assert.equal(
    next.doc.textBetween(next.selection.from, next.selection.to),
    "selected",
  );
});

test("list splitting turns selected hard breaks into separate textblocks", () => {
  const documentNode = doc(para(t("one"), br(), t("two"), br(), t("three")));
  const state = EditorState.create({
    doc: documentNode,
    selection: TextSelection.create(documentNode, 1, 14),
  });

  const transaction = state.tr;
  assert.equal(splitSelectedLinesForListFormatting(transaction), true);
  const next = state.apply(transaction);
  assert.deepEqual(paragraphTexts(next.doc), ["one", "two", "three"]);
});

test("code merge preserves hard breaks", () => {
  const documentNode = doc(para(t("one"), br(), t("two")), para(t("three")));
  const state = EditorState.create({
    doc: documentNode,
    selection: TextSelection.create(
      documentNode,
      1,
      documentNode.content.size - 1,
    ),
  });

  const transaction = state.tr;
  assert.equal(mergeSelectedTextblocksIntoCodeBlock(transaction), true);
  const next = state.apply(transaction);
  assert.equal(next.doc.firstChild.type.name, "codeBlock");
  assert.equal(next.doc.firstChild.textContent, "one\ntwo\nthree");
});

test("code merge preserves meaningful inline atoms and drops unknown leaves", () => {
  const documentNode = doc(
    para(
      t("hello "),
      schema.nodes.mention.create({ label: "@Taylor Ho" }),
      t(" "),
      schema.nodes.customEmoji.create({ shortcode: "party" }),
      schema.nodes.unknownLeaf.create(),
    ),
  );
  const state = EditorState.create({
    doc: documentNode,
    selection: TextSelection.create(
      documentNode,
      1,
      documentNode.content.size - 1,
    ),
  });

  const transaction = state.tr;
  assert.equal(mergeSelectedTextblocksIntoCodeBlock(transaction), true);
  const next = state.apply(transaction);
  assert.equal(next.doc.firstChild.textContent, "hello @Taylor Ho :party:");
  assert.equal(next.doc.firstChild.textContent.includes("secret"), false);
});
