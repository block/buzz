/**
 * `maskMarkdownCode` decides which text `getMentionOffsets` may scan, and
 * `getMentionOffsets` is what the composer uses to attach `p` tags.
 * Every expectation below was checked against react-markdown.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { hasMention } from "./hasMention.ts";

test("a mention on a nested list item is tagged", () => {
  assert.equal(hasMention("- outer\n  - @alice look", "alice"), true);
  assert.equal(hasMention("- outer\n\t- @alice look", "alice"), true);
  assert.equal(hasMention("1. outer\n   1. @alice look", "alice"), true);
});

test("a mention on an indented continuation line is tagged", () => {
  assert.equal(
    hasMention("- a very long item\n  @alice look", "alice"),
    true,
  );
});

test("a real indented code block is still masked", () => {
  assert.equal(
    hasMention("text\n\n    const x = 1;\n    @alice\n", "alice"),
    false,
  );
  assert.equal(hasMention("    @alice", "alice"), false);
  assert.equal(hasMention("text\n\n    a\n\n    @alice", "alice"), false);
});

test("an indented code block that opens without a preceding blank line", () => {
  assert.equal(hasMention("# heading\n    @alice", "alice"), false);
  assert.equal(hasMention("```\nx\n```\n    @alice", "alice"), false);
  assert.equal(hasMention("---\n    @alice", "alice"), false);
});

test("fenced code is unaffected by the indent rule", () => {
  assert.equal(hasMention("```\n@alice\n```", "alice"), false);
  assert.equal(hasMention("```\nx\n```\n@alice", "alice"), true);
});

test("an indented paragraph after a blank line inside a list is tagged", () => {
  assert.equal(hasMention("- item\n\n    @alice", "alice"), true);
});

test("code spans are masked, including across a line ending", () => {
  assert.equal(hasMention("`@alice`", "alice"), false);
  assert.equal(hasMention("see `code @alice` here", "alice"), false);
  assert.equal(hasMention("text `a\n@alice` more", "alice"), false);
  assert.equal(hasMention("` @alice", "alice"), true);
});
