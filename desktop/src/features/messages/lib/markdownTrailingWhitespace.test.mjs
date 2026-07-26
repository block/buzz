import assert from "node:assert/strict";
import test from "node:test";

import { markdownTrailingWhitespace } from "./markdownTrailingWhitespace.ts";

// The markdown parse behind `setContent` drops end-of-line whitespace, which
// silently breaks mention chips (`@Name ` → `@Name`, next keystroke →
// `@Nameabc`, recipient dropped). This decides what gets put back.

test("captures the single trailing space after a mention", () => {
  assert.equal(markdownTrailingWhitespace("@Morgarita "), " ");
});

test("captures the trailing space after multiple mentions", () => {
  assert.equal(markdownTrailingWhitespace("@Vogue @Morgarita "), " ");
});

test("returns null when there is no trailing whitespace", () => {
  assert.equal(markdownTrailingWhitespace("@Morgarita"), null);
  assert.equal(markdownTrailingWhitespace(""), null);
});

test("returns null for whitespace-only input", () => {
  // Nothing to trail. Restoring here would leave a composer that reads as
  // non-empty — Send enabled, placeholder suppressed — on an empty draft.
  assert.equal(markdownTrailingWhitespace(" "), null);
  assert.equal(markdownTrailingWhitespace("   \t"), null);
});

test("captures a mixed run of spaces and tabs", () => {
  assert.equal(markdownTrailingWhitespace("@Morgarita \t "), " \t ");
});

test("captures whitespace trailing the last line only", () => {
  assert.equal(markdownTrailingWhitespace("first line\n@Morgarita "), " ");
});

test("returns null when the string ends on a newline", () => {
  // The caret lands on the empty final line, so nothing needs restoring.
  assert.equal(markdownTrailingWhitespace("@Morgarita \n"), null);
});

test("ignores interior whitespace", () => {
  assert.equal(markdownTrailingWhitespace("@Morgarita  hello"), null);
});
