import assert from "node:assert/strict";
import test from "node:test";

import { isMentionCodeContext } from "./mentionCodeContext.ts";

function textNode(...markNames) {
  return { marks: markNames.map((name) => ({ type: { name } })) };
}

function editor({ active = [], nodeBefore = null, empty = true } = {}) {
  return {
    isActive: (name) => active.includes(name),
    state: { selection: { $from: { nodeBefore }, empty } },
  };
}

test("isMentionCodeContext is false for prose", () => {
  assert.equal(isMentionCodeContext(editor({ nodeBefore: textNode() })), false);
});

test("isMentionCodeContext is false without an editor", () => {
  assert.equal(isMentionCodeContext(null), false);
  assert.equal(isMentionCodeContext(undefined), false);
});

test("isMentionCodeContext detects a fenced code block", () => {
  assert.equal(isMentionCodeContext(editor({ active: ["codeBlock"] })), true);
});

test("isMentionCodeContext detects an inline code span", () => {
  assert.equal(isMentionCodeContext(editor({ active: ["code"] })), true);
});

test("isMentionCodeContext detects a just-closed inline code span", () => {
  // The closing backtick clears the stored mark, so only the text before the
  // caret still reports the code mark.
  assert.equal(
    isMentionCodeContext(editor({ nodeBefore: textNode("code") })),
    true,
  );
});

test("isMentionCodeContext ignores a code span left behind earlier", () => {
  assert.equal(
    isMentionCodeContext(editor({ nodeBefore: textNode("italic") })),
    false,
  );
});

test("isMentionCodeContext ignores the text before a non-empty selection", () => {
  assert.equal(
    isMentionCodeContext(
      editor({ nodeBefore: textNode("code"), empty: false }),
    ),
    false,
  );
});
