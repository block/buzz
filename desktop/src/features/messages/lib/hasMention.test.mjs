import assert from "node:assert/strict";
import test from "node:test";

import { getMentionOffsets, hasMention } from "./hasMention.ts";

test("a possessive still mentions the person", () => {
  // The send path derives the event's p tags from these offsets, so a miss
  // here means no notification — not merely an unstyled name.
  assert.equal(hasMention("@Alice's PR is ready", "Alice"), true);
  assert.equal(hasMention("ping @Alice's bot", "Alice"), true);
});

test("the curly apostrophe macOS substitutes counts too", () => {
  assert.equal(hasMention("@Alice’s PR is ready", "Alice"), true);
});

test("a mention is found at the start, after whitespace, and after '('", () => {
  assert.equal(hasMention("@Alice hi", "Alice"), true);
  assert.equal(hasMention("hi @Alice", "Alice"), true);
  assert.equal(hasMention("(@Alice)", "Alice"), true);
});

test("closing punctuation still ends a mention", () => {
  for (const text of ["@Alice, ok", "@Alice.", "@Alice!", "@Alice?"]) {
    assert.equal(hasMention(text, "Alice"), true, text);
  }
});

test("bold and spoiler wrappers still mention", () => {
  assert.equal(hasMention("**@Alice**", "Alice"), true);
  assert.equal(hasMention("||@Alice||", "Alice"), true);
});

test("an email address is not a mention of its domain", () => {
  assert.equal(hasMention("mail me at bob@Alice.dev", "Alice"), false);
});

test("a mention inside code is not a mention", () => {
  assert.equal(hasMention("`@Alice`", "Alice"), false);
  assert.equal(hasMention("```\n@Alice\n```", "Alice"), false);
});

test("matching is case-insensitive", () => {
  assert.equal(hasMention("@alice hi", "Alice"), true);
});

test("every occurrence gets an offset, in order", () => {
  assert.deepEqual(
    getMentionOffsets("@Alice and @Alice's team", "Alice"),
    [0, 11],
  );
});

test("no mention yields no offsets", () => {
  assert.deepEqual(getMentionOffsets("nothing here", "Alice"), []);
});
