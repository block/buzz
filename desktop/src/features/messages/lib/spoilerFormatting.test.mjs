import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

import { getSpoilerRangeState } from "./spoilerFormatting.ts";
import { SpoilerMark, SPOILER_MARK_NAME } from "./spoilerMark.ts";

const schema = getSchema([
  StarterKit.configure({
    hardBreak: { keepMarks: true },
    heading: false,
    trailingNode: false,
    link: false,
  }),
  SpoilerMark,
]);

const spoilerMark = schema.marks[SPOILER_MARK_NAME];
const codeMark = schema.marks.code;
const para = (...content) => schema.nodes.paragraph.create(null, content);
const codeBlock = (content) => schema.nodes.codeBlock.create(null, content);
const t = (text, marks = []) => schema.text(text, marks);

function doc(...content) {
  return schema.nodes.doc.create(null, content);
}

function wholeDocState(d) {
  return getSpoilerRangeState(d, spoilerMark, 1, d.content.size - 1);
}

test("getSpoilerRangeState: ignores inline code when surrounding text is spoilered", () => {
  const d = doc(
    para(
      t("hidden ", [spoilerMark.create()]),
      t("literal", [codeMark.create()]),
      t(" text", [spoilerMark.create()]),
    ),
  );

  assert.equal(wholeDocState(d), "fully-spoiled");
});

test("getSpoilerRangeState: ignores code blocks when surrounding text is spoilered", () => {
  const d = doc(
    para(t("hidden", [spoilerMark.create()])),
    codeBlock(t("const secret = true;")),
  );

  assert.equal(wholeDocState(d), "fully-spoiled");
});

test("getSpoilerRangeState: reports unspoilered markable text as partial", () => {
  const d = doc(
    para(t("hidden", [spoilerMark.create()])),
    codeBlock(t("const secret = true;")),
    para(t("visible")),
  );

  assert.equal(wholeDocState(d), "partially-spoiled");
});

test("getSpoilerRangeState: returns no markable content for code-only ranges", () => {
  const d = doc(codeBlock(t("const secret = true;")));

  assert.equal(wholeDocState(d), "no-markable-content");
});
