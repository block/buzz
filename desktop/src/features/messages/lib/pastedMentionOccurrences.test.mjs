import assert from "node:assert/strict";
import test from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";

import {
  PastedMentionOccurrencesExtension,
  readPastedMentionOccurrenceRange,
  releasePastedMentionOccurrence,
  trackPastedMentionOccurrence,
} from "./pastedMentionOccurrences.ts";

/**
 * Occurrence ownership, driven through the real ProseMirror plugin.
 *
 * A pasted mention's identity check can outlive the paste, so settlement has
 * to ask whether the text *that paste* inserted is still there. These cases
 * pin what "still" means: positions follow edits elsewhere, and the range dies
 * rather than drifting onto text the user typed — including when the user
 * edits strictly inside it, where both endpoints survive untouched.
 */

const schema = getSchema([
  StarterKit.configure({ heading: false, trailingNode: false, link: false }),
]);
const paragraph = (...content) => schema.nodes.paragraph.create(null, content);
const text = (value) => schema.text(value);
const document = (...content) => schema.nodes.doc.create(null, content);

const PASTED = "@John Smith fixed the bug";

/**
 * A stand-in for `EditorView` over a real `EditorState`.
 *
 * The plugin's `apply` — the whole mechanism under test — runs on every
 * dispatch, without needing a DOM.
 */
function viewWith(initialText, { plugin = true } = {}) {
  const plugins = plugin
    ? PastedMentionOccurrencesExtension.config.addProseMirrorPlugins.call({})
    : [];
  const view = {
    state: EditorState.create({
      doc: document(paragraph(text(initialText))),
      schema,
      plugins,
    }),
    dispatch(tr) {
      view.state = view.state.apply(tr);
    },
  };
  return view;
}

/** Track the whole first paragraph, as a paste into an empty composer does. */
function trackWholeParagraph(view) {
  return trackPastedMentionOccurrence(view, 1, view.state.doc.content.size - 1);
}

/** The text an occurrence still owns, or `null` once its range is gone. */
function ownedText(view, id) {
  const range = readPastedMentionOccurrenceRange(view, id);
  if (!range) return null;
  return view.state.doc.textBetween(range.from, range.to, "\n", "\n");
}

function replaceRange(view, from, to, replacement) {
  const tr = view.state.tr;
  if (replacement === "") tr.delete(from, to);
  else tr.replaceWith(from, to, text(replacement));
  view.dispatch(tr);
}

test("an occurrence still reads its own text after an edit elsewhere", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  // Something typed at the end of the document, past the pasted run.
  view.dispatch(
    view.state.tr.insertText(" thanks", view.state.doc.content.size - 1),
  );

  assert.equal(ownedText(view, id), PASTED);
});

test("an occurrence's positions follow an insertion before it", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  view.dispatch(view.state.tr.insertText("see: ", 1));

  assert.equal(view.state.doc.textContent, `see: ${PASTED}`);
  assert.equal(ownedText(view, id), PASTED);
});

test("text typed at either edge stays outside the occurrence", () => {
  // The paste owns what it inserted and nothing the user appended to it —
  // otherwise a hand-typed label would grow into the range and bind.
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  view.dispatch(view.state.tr.insertText("@Jane Doe ", 1));
  const afterHead = view.state.doc.content.size - 1;
  view.dispatch(view.state.tr.insertText(" @Fizz", afterHead));

  assert.equal(ownedText(view, id), PASTED);
});

test("an occurrence dies when its text is deleted", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  replaceRange(view, 1, view.state.doc.content.size - 1, "");

  assert.equal(ownedText(view, id), null);
});

test("an occurrence dies when the same text is pasted over it", () => {
  // Select-all-and-replace leaves an identical document, so a range that only
  // watched for collapse would survive and claim someone else's paste.
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  replaceRange(view, 1, view.state.doc.content.size - 1, PASTED);

  assert.equal(view.state.doc.textContent, PASTED);
  assert.equal(ownedText(view, id), null);
});

test("an occurrence dies when the composer is cleared on send", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  view.dispatch(view.state.tr.delete(0, view.state.doc.content.size));

  assert.equal(ownedText(view, id), null);
});

test("an occurrence dies when an edit eats into its head or tail", () => {
  const head = viewWith(PASTED);
  const headId = trackWholeParagraph(head);
  replaceRange(head, 1, 2, "");
  assert.equal(ownedText(head, headId), null);

  const tail = viewWith(PASTED);
  const tailId = trackWholeParagraph(tail);
  const end = tail.state.doc.content.size - 1;
  replaceRange(tail, end - 3, end, "");
  assert.equal(ownedText(tail, tailId), null);
});

test("an occurrence dies when an edit replaces text strictly inside it", () => {
  // Neither endpoint moves, and the document ends up character-for-character
  // as it was — so endpoint mapping alone left the range alive and owning
  // words the user had just typed out by hand.
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  const at = 1 + PASTED.indexOf("fixed");
  replaceRange(view, at, at + "fixed".length, "fixed");

  assert.equal(view.state.doc.textContent, PASTED);
  assert.equal(ownedText(view, id), null);
});

test("an occurrence dies when a later step of one transaction overlaps it", () => {
  // The overlap is only visible per step: the first step shifts the range, so
  // a check against the transaction's combined mapping would compare the
  // second step's replaced region to stale coordinates.
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  const tr = view.state.tr;
  tr.insertText("see: ", 1);
  const at = tr.mapping.map(1 + PASTED.indexOf("fixed"));
  tr.replaceWith(at, at + "fixed".length, text("broke"));
  view.dispatch(tr);

  assert.equal(
    view.state.doc.textContent,
    `see: ${PASTED}`.replace("fixed", "broke"),
  );
  assert.equal(ownedText(view, id), null);
});

test("a replacement that only butts an occurrence's boundary spares it", () => {
  // The replaced text was outside the range on either side, so it was never
  // the paste's to lose. Killing here would cost a legitimate identity every
  // time a word beside the paste is edited during verification.
  const head = viewWith(`see: ${PASTED}`);
  const headId = trackPastedMentionOccurrence(
    head,
    1 + "see: ".length,
    head.state.doc.content.size - 1,
  );
  replaceRange(head, 1, 1 + "see: ".length, "");
  assert.equal(ownedText(head, headId), PASTED);

  const tail = viewWith(`${PASTED} thanks`);
  const tailId = trackPastedMentionOccurrence(tail, 1, 1 + PASTED.length);
  replaceRange(
    tail,
    1 + PASTED.length,
    tail.state.doc.content.size - 1,
    " cheers",
  );
  assert.equal(ownedText(tail, tailId), PASTED);
});

test("a pure insertion inside an occurrence leaves it holding the new text", () => {
  // An insertion replaces nothing, so the range legitimately survives — the
  // caller's own check on the surviving text is what refuses a token the user
  // has typed into.
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  view.dispatch(view.state.tr.insertText("X", 1 + "@John".length));

  assert.equal(ownedText(view, id), "@JohnX Smith fixed the bug");
});

test("releasing an occurrence retires it", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);

  releasePastedMentionOccurrence(view, id);

  assert.equal(ownedText(view, id), null);
  // Idempotent: a second release dispatches nothing and does not throw.
  releasePastedMentionOccurrence(view, id);
});

test("occurrences are independent", () => {
  const view = viewWith(`${PASTED} and @Fizz agrees`);
  const first = trackPastedMentionOccurrence(view, 1, 1 + PASTED.length);
  const second = trackPastedMentionOccurrence(
    view,
    1 + PASTED.length,
    view.state.doc.content.size - 1,
  );

  replaceRange(view, 1, 1 + PASTED.length, "");

  assert.equal(ownedText(view, first), null);
  assert.equal(ownedText(view, second), " and @Fizz agrees");
});

test("tracks nothing without a plugin, or for an empty insertion", () => {
  // Fail closed: a composer with no occurrence plugin, or a paste that
  // inserted nothing, has no range a settlement could be held to.
  const unregistered = viewWith(PASTED, { plugin: false });
  assert.equal(trackWholeParagraph(unregistered), null);

  const view = viewWith(PASTED);
  assert.equal(trackPastedMentionOccurrence(view, 3, 3), null);
  assert.equal(ownedText(view, null), null);
});

test("tracks nothing for a destroyed view", () => {
  const view = viewWith(PASTED);
  const id = trackWholeParagraph(view);
  view.isDestroyed = true;

  assert.equal(ownedText(view, id), null);
  assert.equal(trackWholeParagraph(view), null);
});
