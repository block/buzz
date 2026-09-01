/**
 * `maskMarkdownCode` decides which text `getMentionOffsets` may scan, and
 * `getMentionOffsets` is what the composer uses to attach `p` tags
 * (`draftMentionRefs.ts`, `extractMentionPubkeys.ts`). Both directions are
 * user-visible: a line masked wrongly is a mention the sender's own client
 * never tags, and a line left unmasked notifies people — and wakes agents —
 * for text the UI displays inside `<code>`.
 *
 * Every expectation below was checked against react-markdown, the renderer
 * that decides what the reader actually sees.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { hasMention } from "./hasMention.ts";

test("a mention on a nested list item is tagged", () => {
  // react-markdown renders all of these as an ordinary nested bullet, not as
  // code. Masking them dropped the p tag from the most common markdown shape
  // that carries four spaces of indent.
  assert.equal(hasMention("- outer\n    - @alice look", "alice"), true);
  assert.equal(hasMention("- outer\n\t- @alice look", "alice"), true);
  assert.equal(hasMention("1. outer\n    1. @alice look", "alice"), true);
});

test("a mention on an indented continuation line is tagged", () => {
  assert.equal(
    hasMention("- a very long item\n    @alice look", "alice"),
    true,
  );
});

test("a real indented code block is still masked", () => {
  // Blank line first, so this one genuinely is code — and react-markdown
  // agrees, rendering it inside <pre><code>.
  assert.equal(
    hasMention("text\n\n    const x = 1;\n    @alice\n", "alice"),
    false,
  );
  // At the very start of a message there is nothing to interrupt.
  assert.equal(hasMention("    @alice", "alice"), false);
  // A blank line inside a chunk does not end it.
  assert.equal(hasMention("text\n\n    a\n\n    @alice", "alice"), false);
});

test("an indented code block that opens without a preceding blank line", () => {
  // An indented code block cannot interrupt a *paragraph*, but it needs no
  // blank line after a block that cannot be continued. react-markdown puts
  // @alice inside <pre><code> in each of these; a line-state rule keyed on
  // "previous line was blank" reports a visible mention instead.
  assert.equal(hasMention("# heading\n    @alice", "alice"), false);
  assert.equal(hasMention("```\nx\n```\n    @alice", "alice"), false);
  assert.equal(hasMention("---\n    @alice", "alice"), false);
});

test("fenced code is unaffected by the indent rule", () => {
  assert.equal(hasMention("```\n@alice\n```", "alice"), false);
  assert.equal(hasMention("```\nx\n```\n@alice", "alice"), true);
  // Indented code, then a fence, then an indented line: still code.
  assert.equal(hasMention("    code\n```\nx\n```\n    @alice", "alice"), false);
});

test("an indented paragraph after a blank line inside a list is tagged", () => {
  // CommonMark reads this as the list item's second paragraph — the indent is
  // consumed by the list, leaving nothing to start a code block — so
  // react-markdown renders the mention visibly. Telling this apart from a real
  // indented code block needs the list's content indent, which is why the
  // masker asks the parser instead of classifying lines itself.
  assert.equal(hasMention("- item\n\n    @alice", "alice"), true);
});

test("code spans are masked, including across a line ending", () => {
  assert.equal(hasMention("`@alice`", "alice"), false);
  assert.equal(hasMention("see `code @alice` here", "alice"), false);
  assert.equal(hasMention("text `a\n@alice` more", "alice"), false);
  // An unterminated opener is literal text, so the mention is visible.
  assert.equal(hasMention("` @alice", "alice"), true);
});
