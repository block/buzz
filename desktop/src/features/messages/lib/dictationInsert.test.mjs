/**
 * Tests for the dictation caret-insertion spacing rule.
 *
 * The STT pipeline emits one trimmed segment per speech burst with no
 * inter-segment spacing, so the join is entirely this function's job. These
 * exercise the ACTUAL exported helper the toolbar calls, so a regression in
 * the spacing logic breaks here rather than silently shipping "therehow".
 */

import assert from "node:assert/strict";
import test from "node:test";

import { buildDictationInsert } from "./dictationInsert.ts";

test("first segment in an empty composer gets no leading space", () => {
  assert.equal(buildDictationInsert("", "hello there"), "hello there");
});

test("a following segment is separated from the previous word", () => {
  assert.equal(
    buildDictationInsert("hello there", "how are you"),
    " how are you",
  );
});

test("existing trailing whitespace is not doubled", () => {
  assert.equal(buildDictationInsert("hello ", "there"), "there");
});

test("a trailing newline counts as whitespace", () => {
  assert.equal(
    buildDictationInsert("first line\n", "second line"),
    "second line",
  );
});

test("segment whitespace is trimmed before insertion", () => {
  assert.equal(buildDictationInsert("hello", "  there  "), " there");
});

test("a whitespace-only segment inserts nothing", () => {
  assert.equal(buildDictationInsert("hello", "   "), "");
  assert.equal(buildDictationInsert("", ""), "");
});

test("punctuation before the caret still takes a separating space", () => {
  assert.equal(buildDictationInsert("Hello.", "How are you"), " How are you");
});
