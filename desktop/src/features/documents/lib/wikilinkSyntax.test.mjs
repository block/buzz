import assert from "node:assert/strict";
import { test } from "node:test";

import {
  extractLinkTargets,
  parseWikilinks,
  wikilinkDisplayText,
} from "./wikilinkSyntax.ts";

/** Compact shape for assertions. */
function shape(link) {
  return [link.target, link.heading, link.blockId, link.alias];
}

test("parses all six Obsidian wikilink forms", () => {
  const cases = [
    ["[[Note]]", ["Note", null, null, null]],
    ["[[Note|alias]]", ["Note", null, null, "alias"]],
    ["[[Note#Heading]]", ["Note", "Heading", null, null]],
    ["[[Note#Heading|alias]]", ["Note", "Heading", null, "alias"]],
    ["[[Note^blockid]]", ["Note", null, "blockid", null]],
    ["[[Note#^blockid]]", ["Note", null, "blockid", null]],
  ];
  for (const [source, expected] of cases) {
    const [link] = parseWikilinks(source);
    assert.ok(link, `${source} did not parse`);
    assert.deepEqual(shape(link), expected, source);
  }
});

test("parses same-note anchors with an empty target", () => {
  assert.deepEqual(shape(parseWikilinks("[[#Heading]]")[0]), [
    "",
    "Heading",
    null,
    null,
  ]);
  assert.deepEqual(shape(parseWikilinks("[[^blockid]]")[0]), [
    "",
    null,
    "blockid",
    null,
  ]);
});

test("#^ is read as a block reference, not a heading named ^id", () => {
  // Ordering bug bait: matching bare `#` first would capture "^blockid".
  const [link] = parseWikilinks("[[Note#^blockid]]");
  assert.equal(link.blockId, "blockid");
  assert.equal(link.heading, null);
});

test("tolerates the escaped brackets the serializer emits", () => {
  // prosemirror-markdown escapes `[` and `]` in text nodes, so a link read
  // back out of the editor arrives looking like this.
  const [link] = parseWikilinks("A \\[\\[Note Title\\]\\] reference.");
  assert.deepEqual(shape(link), ["Note Title", null, null, null]);

  const [aliased] = parseWikilinks(
    "\\[\\[Note\\|alias\\]\\]".replace("\\|", "|"),
  );
  assert.equal(aliased.target, "Note");
});

test("does not match embeds", () => {
  // `![[...]]` is transclusion, a different construct.
  assert.deepEqual(parseWikilinks("![[Some Note]]"), []);
  assert.deepEqual(parseWikilinks("!\\[\\[Some Note\\]\\]"), []);
  // But an embed adjacent to a real link must not suppress the link.
  const links = parseWikilinks("![[Embed]] and [[Real]]");
  assert.deepEqual(
    links.map((link) => link.target),
    ["Real"],
  );
});

test("finds several links in one line and reports their offsets", () => {
  const source = "See [[One]] and [[Two|second]].";
  const links = parseWikilinks(source);
  assert.deepEqual(
    links.map((link) => link.target),
    ["One", "Two"],
  );
  assert.equal(
    source.slice(links[0].index, links[0].index + links[0].raw.length),
    "[[One]]",
  );
  assert.equal(links[1].alias, "second");
});

test("does not span newlines", () => {
  // An unclosed `[[` must not swallow the rest of the document.
  assert.deepEqual(
    parseWikilinks("[[Unclosed\n\nOther [[Real]]").map((l) => l.target),
    ["Real"],
  );
});

test("ignores a link with no destination at all", () => {
  assert.deepEqual(parseWikilinks("[[]]"), []);
  assert.deepEqual(parseWikilinks("[[   ]]"), []);
});

test("trims incidental whitespace inside the brackets", () => {
  const [link] = parseWikilinks("[[  Note Title  |  alias  ]]");
  assert.equal(link.target, "Note Title");
  assert.equal(link.alias, "alias");
});

test("extractLinkTargets dedupes and drops same-note anchors", () => {
  const source = "[[One]] [[One]] [[Two]] [[#Heading]] [[^block]]";
  // A note must not appear to link to itself just for having anchors.
  assert.deepEqual(extractLinkTargets(source), ["One", "Two"]);
});

test("repeated calls do not leak regex state", () => {
  // A shared global-flagged regex would return [] on the second call.
  const source = "[[One]] [[Two]]";
  assert.equal(parseWikilinks(source).length, 2);
  assert.equal(parseWikilinks(source).length, 2);
});

test("display text prefers the alias, then target and heading", () => {
  assert.equal(
    wikilinkDisplayText(parseWikilinks("[[Note|shown]]")[0]),
    "shown",
  );
  assert.equal(wikilinkDisplayText(parseWikilinks("[[Note]]")[0]), "Note");
  assert.equal(
    wikilinkDisplayText(parseWikilinks("[[Note#Heading]]")[0]),
    "Note › Heading",
  );
  assert.equal(
    wikilinkDisplayText(parseWikilinks("[[#Heading]]")[0]),
    "Heading",
  );
  assert.equal(wikilinkDisplayText(parseWikilinks("[[^block]]")[0]), "block");
});
