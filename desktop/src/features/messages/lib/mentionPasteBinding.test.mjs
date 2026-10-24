import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { getSchema } from "@tiptap/core";
import { EditorState } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";
import { JSDOM } from "jsdom";

import { extractMentionPubkeys } from "./extractMentionPubkeys.ts";
import { PastedMentionOccurrencesExtension } from "./pastedMentionOccurrences.ts";

/**
 * The three fences a settled paste has to clear, driven through the hook the
 * composers actually use.
 *
 * Verification is deferred by hand here rather than timed, so each case pins
 * an ordering rather than a race: a paste whose answer is still outstanding, a
 * newer intent for the same label, and a mention token the user has since
 * edited. The mention map and `extractMentionPubkeys` are the real ones
 * `useMentions` writes to and reads with, so what these assert is what a send
 * would put in its `p` tags.
 *
 * The occurrence fence is the `@Label` token rather than the whole insertion,
 * and that boundary cuts both ways: rewriting the mention itself must cost the
 * paste its identity even though the sentence around it is untouched, and
 * rewriting the sentence must *not*, since the slow-verifying non-member case
 * is the one the feature exists for.
 */

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** 64-hex, the only shape `parseMentionClipboardRecords` lets through. */
const KEY_A = "a".repeat(64);
const KEY_B = "b".repeat(64);

const PASTED = "@John Smith fixed the bug";
const SECOND_PASTE = " and @John Smith agrees";
const TOKEN = "@John Smith";
/** A paste with text either side of its mention, as most copies have. */
const SENTENCE = `Hello ${TOKEN} fixed the bug`;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const schema = getSchema([
  StarterKit.configure({ heading: false, trailingNode: false, link: false }),
]);
const text = (value) => schema.text(value);
const document_ = (value) =>
  schema.nodes.doc.create(null, [
    schema.nodes.paragraph.create(null, [text(value)]),
  ]);

/** A stand-in for `EditorView` over a real `EditorState` and real plugin. */
function viewWith(initialText) {
  const view = {
    state: EditorState.create({
      doc: document_(initialText),
      schema,
      plugins:
        PastedMentionOccurrencesExtension.config.addProseMirrorPlugins.call({}),
    }),
    dispatch(tr) {
      view.state = view.state.apply(tr);
    },
  };
  return view;
}

/** Clipboard HTML in the shape a Buzz copy writes. */
function clipboardHtml(label, pubkey, body) {
  return (
    '<span data-buzz-copy="markdown">' +
    `<span data-mention="" data-mention-pubkey="${pubkey}" ` +
    `data-mention-label="${label}">@${label}</span>` +
    `${body}</span>`
  );
}

function deferred() {
  let resolve;
  const promise = new Promise((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

/**
 * Render the binder with the mention map `useMentions` keeps, and a verifier
 * whose answers the test releases one at a time.
 */
async function renderBinder() {
  const { renderHook } = await import("@testing-library/react");
  const { useMentionPasteBinding } = await import("./mentionPasteBinding.ts");

  /** Stands in for `mentionMapRef.current`; the writer mirrors the hook's. */
  const mentionMap = new Map();
  const answers = [];
  const { result } = renderHook(() =>
    useMentionPasteBinding({
      registerVerifiedMentionPubkey: (displayName, pubkey) => {
        mentionMap.set(displayName.trim(), pubkey);
      },
      verifyMentionIdentities: () => {
        const next = deferred();
        answers.push(next);
        return next.promise;
      },
    }),
  );

  return {
    /** The pubkeys a send would tag for `body`. */
    extract: (body) =>
      extractMentionPubkeys({
        text: body,
        selectedMentions: mentionMap,
        selectedDisplayNames: [],
        memberCandidates: [],
      }),
    mentionMap,
    /** Answer the nth outstanding verification, oldest first. */
    vouch: (index, identities) => answers[index].resolve(identities),
    get binding() {
      return result.current;
    },
  };
}

/** Paste `body`'s records into `view` over the range production hands over. */
function paste(binding, view, { label, pubkey, body, from, to }) {
  binding.bindPastedMentionIdentities({
    html: clipboardHtml(label, pubkey, body.slice(`@${label}`.length)),
    insertedText: body,
    insertedRange: { from, to },
    view,
  });
}

/** Paste a whole paragraph the view was built holding. */
function pasteWholeParagraph(binding, view, { label, pubkey, body }) {
  paste(binding, view, {
    label,
    pubkey,
    body,
    from: 1,
    to: 1 + body.length,
  });
}

/** Replace `[at, at + length)` the way a select-and-retype does. */
function replaceRange(view, at, length, replacement) {
  view.dispatch(view.state.tr.replaceWith(at, at + length, text(replacement)));
}

test("a pasted identity binds only once its verification has settled", async () => {
  // The send seams await `settlePendingMentionBindings` precisely because this
  // window exists: sending inside it publishes a readable label with no tag.
  const harness = await renderBinder();
  const view = viewWith(PASTED);
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: PASTED,
    from: 1,
    to: 1 + PASTED.length,
  });

  assert.deepEqual(harness.extract(PASTED), [], "nothing binds mid-flight");

  const drained = harness.binding.settlePendingMentionBindings();
  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await drained;

  assert.deepEqual(harness.extract(PASTED), [KEY_A]);
});

test("a settled paste does not overwrite a newer paste of the same label", async () => {
  // Slow A, fast B, same label. Both occurrences stay alive, so ordering — not
  // visibility — is the only thing that can decide which pubkey owns the name.
  const harness = await renderBinder();
  const view = viewWith(PASTED + SECOND_PASTE);
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: PASTED,
    from: 1,
    to: 1 + PASTED.length,
  });
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_B,
    body: SECOND_PASTE,
    from: 1 + PASTED.length,
    to: 1 + PASTED.length + SECOND_PASTE.length,
  });

  harness.vouch(1, [{ label: "John Smith", pubkey: KEY_B, isAgent: false }]);
  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(PASTED), [KEY_B]);
});

test("an explicit selection outranks a paste still being verified", async () => {
  // What the picker and every other `registerMentionPubkey` caller do: claim
  // the label, then write it. A paste that resolves afterwards is stale.
  const harness = await renderBinder();
  const view = viewWith(PASTED);
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: PASTED,
    from: 1,
    to: 1 + PASTED.length,
  });

  harness.binding.claimMentionIntent("John Smith");
  harness.mentionMap.set("John Smith", KEY_B);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(PASTED), [KEY_B]);
});

test("a paste whose text is gone binds nothing, label elsewhere or not", async () => {
  // Delete the paste, then write the same name by hand. "Is this label in the
  // composer?" says yes; the paste no longer owns any of it.
  const harness = await renderBinder();
  const view = viewWith(PASTED);
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: PASTED,
    from: 1,
    to: 1 + PASTED.length,
  });

  const pastedEnd = 1 + PASTED.length;
  view.dispatch(view.state.tr.insertText(SECOND_PASTE, pastedEnd));
  view.dispatch(view.state.tr.delete(1, pastedEnd));
  assert.equal(view.state.doc.textContent, SECOND_PASTE);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(SECOND_PASTE), []);
});

test("rewriting the pasted mention itself binds nothing to the typed words", async () => {
  // Select exactly the mention and type it out again. The edit is strictly
  // inside the paste, so both of the insertion's endpoints survive and its
  // text is character-for-character what it was — the whole-insertion fence
  // saw nothing at all, and handed the clipboard's key to hand-typed words.
  const harness = await renderBinder();
  const view = viewWith(SENTENCE);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: SENTENCE,
  });

  replaceRange(view, 1 + SENTENCE.indexOf(TOKEN), TOKEN.length, TOKEN);
  assert.equal(view.state.doc.textContent, SENTENCE);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(SENTENCE), []);
});

test("editing a word beside the pasted mention keeps its identity", async () => {
  // The payoff for fencing the token rather than the insertion: a lookup that
  // crosses the network is exactly the case the user has time to tidy the
  // sentence during, and tidying it must not silently cost the mention.
  const harness = await renderBinder();
  const view = viewWith(SENTENCE);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: SENTENCE,
  });

  const at = 1 + SENTENCE.indexOf("fixed ");
  view.dispatch(view.state.tr.delete(at, at + "fixed ".length));
  const edited = SENTENCE.replace("fixed ", "");
  assert.equal(view.state.doc.textContent, edited);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(edited), [KEY_A]);
});

test("rewriting one occurrence leaves the label its other one", async () => {
  // Each occurrence is fenced on its own, and the name is still on screen off
  // this paste — so one of them being retyped is not the label's whole claim.
  const body = `${TOKEN} and ${TOKEN} again`;
  const harness = await renderBinder();
  const view = viewWith(body);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body,
  });

  replaceRange(view, 1, TOKEN.length, TOKEN);
  assert.equal(view.state.doc.textContent, body);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(body), [KEY_A]);
});

test("rewriting part of the pasted mention binds nothing", async () => {
  // Select the first name inside the token and type it again. Neither of the
  // token's endpoints is touched, so endpoint mapping alone reports a whole
  // live mention over characters the user wrote — and the text check agrees
  // with it, because the document reads exactly as it did.
  const harness = await renderBinder();
  const view = viewWith(SENTENCE);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: SENTENCE,
  });

  replaceRange(view, 1 + SENTENCE.indexOf("John"), "John".length, "John");
  assert.equal(view.state.doc.textContent, SENTENCE);

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(SENTENCE), []);
});

test("typing inside the pasted mention binds nothing", async () => {
  // A pure insertion replaces nothing, so the tracked range survives it by
  // design. What refuses this is the token's own text no longer reading as a
  // mention of the label the clipboard named.
  const harness = await renderBinder();
  const view = viewWith(SENTENCE);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: SENTENCE,
  });

  view.dispatch(view.state.tr.insertText("y", 1 + SENTENCE.indexOf("Smith")));
  assert.equal(
    view.state.doc.textContent,
    SENTENCE.replace(TOKEN, "@John ySmith"),
  );

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  // The map entry is the real damage: nothing shows the label now, but a
  // binding outlives its paste and would light the next one typed by hand.
  assert.equal(harness.mentionMap.has("John Smith"), false);
});

test("typing against the pasted mention's edge binds nothing", async () => {
  // Text typed at either edge lands outside the tracked range on purpose —
  // and it is exactly the text that destroys the word boundary a mention
  // needs, so the range reads whole while the document shows no mention.
  const harness = await renderBinder();
  const view = viewWith(SENTENCE);
  pasteWholeParagraph(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: SENTENCE,
  });

  view.dispatch(
    view.state.tr.insertText("x", 1 + SENTENCE.indexOf(TOKEN) + TOKEN.length),
  );
  assert.equal(
    view.state.doc.textContent,
    SENTENCE.replace(TOKEN, `${TOKEN}x`),
  );

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.equal(harness.mentionMap.has("John Smith"), false);
});

test("clearing the composer's mentions retires an in-flight paste", async () => {
  const harness = await renderBinder();
  const view = viewWith(PASTED);
  paste(harness.binding, view, {
    label: "John Smith",
    pubkey: KEY_A,
    body: PASTED,
    from: 1,
    to: 1 + PASTED.length,
  });

  harness.binding.clearMentionIntents();

  harness.vouch(0, [{ label: "John Smith", pubkey: KEY_A, isAgent: false }]);
  await harness.binding.settlePendingMentionBindings();

  assert.deepEqual(harness.extract(PASTED), []);
});

test("a hidden record costs no verification and binds nothing", async () => {
  const harness = await renderBinder();
  const view = viewWith("look at this");
  harness.binding.bindPastedMentionIdentities({
    html:
      `<span data-mention="" data-mention-pubkey="${KEY_A}" ` +
      'data-mention-label="John Smith"></span>look at this',
    insertedText: "look at this",
    insertedRange: { from: 1, to: 1 + "look at this".length },
    view,
  });

  // Nothing to settle: the sync visibility gate declined it before any lookup.
  await harness.binding.settlePendingMentionBindings();
  assert.deepEqual(harness.extract(PASTED), []);
});

test("draining is a no-op when nothing is pending", async () => {
  const harness = await renderBinder();
  await harness.binding.settlePendingMentionBindings();
});
