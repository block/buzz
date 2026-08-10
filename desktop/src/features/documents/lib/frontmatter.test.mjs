import assert from "node:assert/strict";
import { test } from "node:test";

import { joinFrontmatter, splitFrontmatter } from "./frontmatter.ts";

test("splits a frontmatter block off the body", () => {
  const raw = "---\ntitle: Note\ntags: [a, b]\n---\n\n# Body\n";
  const split = splitFrontmatter(raw);
  // The blank separator line belongs to the frontmatter side: the editor drops
  // a leading blank line, so keeping it on the body would fail the round-trip
  // guard over pure cosmetics.
  assert.equal(split.frontmatter, "---\ntitle: Note\ntags: [a, b]\n---\n\n");
  assert.equal(split.body, "# Body\n");
});

test("absorbs however many blank lines follow the closing fence", () => {
  const { body, frontmatter } = splitFrontmatter(
    "---\na: 1\n---\n\n\n\n# Body",
  );
  assert.equal(frontmatter, "---\na: 1\n---\n\n\n\n");
  assert.equal(body, "# Body");
});

test("a body starting immediately after the fence keeps no separator", () => {
  const { body, frontmatter } = splitFrontmatter("---\na: 1\n---\n# Body");
  assert.equal(frontmatter, "---\na: 1\n---\n");
  assert.equal(body, "# Body");
});

test("round-trips byte-for-byte through join", () => {
  for (const raw of [
    "---\ntitle: Note\n---\n\n# Body\n",
    "# No frontmatter\n",
    "---\ntitle: Note\n---\n",
    "---\r\ntitle: CRLF\r\n---\r\n\r\n# Body\r\n",
    "---\ntricky: 'value with --- inside'\n---\n\nbody",
  ]) {
    const { body, frontmatter } = splitFrontmatter(raw);
    assert.equal(joinFrontmatter(frontmatter, body), raw, raw);
  }
});

test("a file with no frontmatter is all body", () => {
  const raw = "# Just a heading\n\nSome text.\n";
  assert.deepEqual(splitFrontmatter(raw), { body: raw, frontmatter: null });
});

test("a thematic break partway down the file is not frontmatter", () => {
  // The opening fence must be the very first line.
  const raw = "# Heading\n\n---\n\nMore text.\n";
  assert.deepEqual(splitFrontmatter(raw), { body: raw, frontmatter: null });
});

test("an unterminated block is treated as body, not swallowed", () => {
  // Losing the file because someone typed `---` on line 1 would be far worse
  // than declining to split.
  const raw = "---\ntitle: never closed\n\n# Body\n";
  assert.deepEqual(splitFrontmatter(raw), { body: raw, frontmatter: null });
});

test("tolerates CRLF delimiters", () => {
  const raw = "---\r\ntitle: Note\r\n---\r\n\r\n# Body\r\n";
  const { body, frontmatter } = splitFrontmatter(raw);
  assert.equal(frontmatter, "---\r\ntitle: Note\r\n---\r\n\r\n");
  assert.equal(body, "# Body\r\n");
});

test("an empty frontmatter block still splits", () => {
  const raw = "---\n---\n\nbody\n";
  const { body, frontmatter } = splitFrontmatter(raw);
  assert.equal(frontmatter, "---\n---\n\n");
  assert.equal(body, "body\n");
});

test("joinFrontmatter with no frontmatter returns the body unchanged", () => {
  assert.equal(joinFrontmatter(null, "# Body"), "# Body");
});
