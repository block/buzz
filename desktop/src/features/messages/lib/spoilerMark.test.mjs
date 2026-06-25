import assert from "node:assert/strict";
import test from "node:test";

import { findClosingDelimiter } from "./spoilerMark.ts";

test("findClosingDelimiter: closes on the first inner delimiter", () => {
  const source = "||a || b||";

  assert.equal(findClosingDelimiter(source, 2, source.length), 4);
});

test("findClosingDelimiter: returns -1 when no closing delimiter exists", () => {
  const source = "||open spoiler";

  assert.equal(findClosingDelimiter(source, 2, source.length), -1);
});
