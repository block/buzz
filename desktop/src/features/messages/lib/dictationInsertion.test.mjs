import assert from "node:assert/strict";
import test from "node:test";

import { buildDictationInsertion } from "./dictationInsertion.ts";

test("inserts the first transcript without a leading space", () => {
  assert.equal(
    buildDictationInsertion("", "  Hello from voice.  "),
    "Hello from voice. ",
  );
});

test("separates dictation from existing text", () => {
  assert.equal(
    buildDictationInsertion("d", "Continue this thought."),
    " Continue this thought. ",
  );
});

test("does not double-space at an existing boundary", () => {
  assert.equal(
    buildDictationInsertion(" ", "Continue this thought."),
    "Continue this thought. ",
  );
});

test("ignores an empty recognition result", () => {
  assert.equal(buildDictationInsertion("x", " \n "), "");
});
