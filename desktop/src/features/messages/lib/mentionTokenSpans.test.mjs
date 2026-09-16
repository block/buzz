import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";

import { findMentionTokenSpans } from "./mentionTokenSpans.ts";

/**
 * Mention tokens located in document coordinates.
 *
 * The fence on a pasted identity is the range its `@Label` occupies, so these
 * pin the conversion both ways: a span has to cover the sigil and the whole
 * label and nothing else, and it has to be a position the editor agrees with —
 * every case reads the span back out of the document and compares text.
 */

const schema = getSchema([
  StarterKit.configure({ heading: false, trailingNode: false, link: false }),
]);
const text = (value) => schema.text(value);
const paragraph = (...content) => schema.nodes.paragraph.create(null, content);
const document_ = (...content) => schema.nodes.doc.create(null, content);

const LABEL = "John Smith";
const TOKEN = `@${LABEL}`;

/** What the document actually holds at each returned span. */
function spanTexts(doc, spans) {
  return spans.map((span) => doc.textBetween(span.from, span.to, "\n", "\n"));
}

function wholeDoc(doc) {
  return { from: 0, to: doc.content.size };
}

test("a span covers the sigil and the whole label", () => {
  const doc = document_(paragraph(text(`Hello ${TOKEN} fixed the bug`)));

  const spans = findMentionTokenSpans(doc, wholeDoc(doc), [LABEL]);

  assert.equal(spans.length, 1);
  assert.deepEqual(spanTexts(doc, spans), [TOKEN]);
  assert.equal(spans[0].label, LABEL);
});

test("spans come back in document order across labels", () => {
  const doc = document_(paragraph(text(`@Fizz and ${TOKEN} and @Fizz again`)));

  const spans = findMentionTokenSpans(doc, wholeDoc(doc), ["Fizz", LABEL]);

  assert.deepEqual(spanTexts(doc, spans), ["@Fizz", TOKEN, "@Fizz"]);
});

test("positions survive a block boundary before the mention", () => {
  // The second paragraph's characters sit past its own node position, so a
  // span read off a flat offset would land a character or two to the left.
  const doc = document_(
    paragraph(text("first line")),
    paragraph(text(`then ${TOKEN} replied`)),
  );

  const spans = findMentionTokenSpans(doc, wholeDoc(doc), [LABEL]);

  assert.deepEqual(spanTexts(doc, spans), [TOKEN]);
});

test("a mention split across a block boundary yields no span", () => {
  // The separator makes the run read as a mention of nothing contiguous;
  // there is no single range to fence, so there is nothing to bind.
  const doc = document_(paragraph(text("@John")), paragraph(text("Smith")));

  assert.deepEqual(findMentionTokenSpans(doc, wholeDoc(doc), [LABEL]), []);
});

test("marks inside the label do not split its span", () => {
  // TipTap parses `**@John** Smith` into two text nodes; the token is still
  // one contiguous run of characters, and the fence has to hold all of it.
  const bold = schema.marks.bold.create();
  const doc = document_(
    paragraph(text("@John", [bold]), text(" Smith fixed the bug")),
  );

  const spans = findMentionTokenSpans(doc, wholeDoc(doc), [LABEL]);

  assert.deepEqual(spanTexts(doc, spans), [TOKEN]);
});

test("only mentions inside the range are returned", () => {
  const doc = document_(paragraph(text(`${TOKEN} and ${TOKEN} again`)));
  const second = 1 + `${TOKEN} and `.length;

  const spans = findMentionTokenSpans(
    doc,
    { from: second, to: doc.content.size },
    [LABEL],
  );

  assert.equal(spans.length, 1);
  assert.equal(spans[0].from, second);
  assert.deepEqual(spanTexts(doc, spans), [TOKEN]);
});

test("a range that cuts the label short yields no span", () => {
  // Fail closed: half a token is not the mention the clipboard named.
  const doc = document_(paragraph(text(`${TOKEN} fixed the bug`)));

  assert.deepEqual(
    findMentionTokenSpans(doc, { from: 1, to: 1 + "@John Sm".length }, [LABEL]),
    [],
  );
});

test("text with no word boundary after the label yields no span", () => {
  // The same rule every other mention layer applies: `@John Smithx` names
  // nobody, so nothing there can carry an identity.
  const doc = document_(paragraph(text(`${TOKEN}x fixed the bug`)));

  assert.deepEqual(findMentionTokenSpans(doc, wholeDoc(doc), [LABEL]), []);
});

test("an empty or inverted range yields no spans", () => {
  const doc = document_(paragraph(text(`${TOKEN} fixed the bug`)));

  assert.deepEqual(findMentionTokenSpans(doc, { from: 5, to: 5 }, [LABEL]), []);
  assert.deepEqual(findMentionTokenSpans(doc, { from: 9, to: 2 }, [LABEL]), []);
});

test("a range past the document is clamped rather than trusted", () => {
  // Settlement asks for a character either side of a tracked range, which at
  // the document's edge is a position that does not exist.
  const doc = document_(paragraph(text(TOKEN)));

  const spans = findMentionTokenSpans(
    doc,
    { from: -5, to: doc.content.size + 5 },
    [LABEL],
  );

  assert.deepEqual(spanTexts(doc, spans), [TOKEN]);
});
