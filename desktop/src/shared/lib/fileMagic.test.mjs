import assert from "node:assert/strict";
import test from "node:test";

import { isSingleItemFile } from "./fileMagic.ts";

test("isSingleItemFile treats markdown persona files as single imports", () => {
  assert.equal(isSingleItemFile([45, 45, 45], "fizz.md"), true);
});

test("isSingleItemFile treats legacy persona markdown files as single imports", () => {
  assert.equal(isSingleItemFile([45, 45, 45], "fizz.persona.md"), true);
});

test("isSingleItemFile does not classify arbitrary text files as single imports", () => {
  assert.equal(isSingleItemFile([45, 45, 45], "notes.txt"), false);
});
