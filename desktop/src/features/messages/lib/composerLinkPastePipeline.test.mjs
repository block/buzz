/**
 * Composed-editor regression coverage for selected-text link paste.
 *
 * The unit tests in `composerMessageLinkNode.test.mjs` drive
 * `createComposerLinkPasteHandler` against a hand-built schema and a mock
 * view — they cannot see what happens *after* the handler declines. That gap
 * hid a real bug: `false` from `editorProps.handleDOMEvents.paste` does not end
 * paste handling, it hands the event to ProseMirror's built-in paste, which
 * runs every plugin's `handlePaste`. TipTap's Link plugin recognised the same
 * URLs one layer down and partially linked selections the composer had
 * deliberately refused.
 *
 * So these tests build the *production* editor from `useRichTextEditor` and
 * dispatch a real DOM `paste` event at `view.dom`. If someone re-enables
 * `linkOnPaste` or slots another URL-aware plugin into the chain, this fails.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { find as findLinks } from "linkifyjs";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const CHANNEL_HREF = `buzz://channel/${CHANNEL_ID}`;
const MESSAGE_LINK_CHANNELS = [{ id: CHANNEL_ID, name: "general" }];

before(() => {
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  // `navigator` is a getter-only global from Node 21 on, so Object.assign
  // throws on it. prosemirror-view reads userAgent for browser quirks.
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  Object.assign(globalThis, {
    ClipboardEvent: dom.window.Event,
    CustomEvent: dom.window.CustomEvent,
    DOMParser: dom.window.DOMParser,
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    Range: dom.window.Range,
    ResizeObserver: class {
      disconnect() {}
      observe() {}
      unobserve() {}
    },
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

/**
 * Mounts the production composer editor and returns its Tiptap instance.
 */
async function mountComposerEditor() {
  const React = await import("react");
  const { act, render, waitFor } = await import("@testing-library/react");
  const { EditorContent } = await import("@tiptap/react");
  const { useRichTextEditor } = await import("./useRichTextEditor.ts");

  let editor = null;
  function Harness() {
    const instance = useRichTextEditor({
      messageLinkChannels: MESSAGE_LINK_CHANNELS,
    }).editor;
    editor = instance;
    return instance
      ? React.createElement(EditorContent, { editor: instance })
      : null;
  }

  await act(async () => {
    render(React.createElement(Harness));
  });
  // Tiptap emits `create` from a `setTimeout(…, 0)`, and Link's `onCreate` is
  // what teaches linkify the `buzz` protocol. Paste before that lands and a
  // `buzz://` assertion passes for the wrong reason.
  await waitFor(() =>
    assert.ok(editor?.isInitialized, "composer editor never emitted `create`"),
  );

  // Check that precondition rather than trust the wait. It has to be checked
  // *after* `create`, never as the poll itself: `find` initialises linkify's
  // scanner on first call, and `registerCustomProtocol` after that only warns,
  // so polling on `find` would break the registration it is watching for.
  assert.equal(
    findLinks(CHANNEL_HREF)[0]?.href,
    CHANNEL_HREF,
    "expected Link's onCreate to register the buzz protocol with linkify",
  );
  return editor;
}

const paragraph = (...content) => ({ type: "paragraph", content });
const codeBlock = (text) => ({
  type: "codeBlock",
  content: [{ type: "text", text }],
});
const text = (value, marks) => ({
  type: "text",
  text: value,
  ...(marks && { marks }),
});

/**
 * Seeds the document from ProseMirror JSON. Not an HTML string —
 * `tiptap-markdown` parses `setContent` input as Markdown, so HTML arrives as
 * literal text and the test silently exercises the wrong document.
 */
function seedDocument(editor, ...content) {
  editor.commands.setContent({ type: "doc", content });
}

function selectAll(editor) {
  editor.commands.setTextSelection({
    from: 0,
    to: editor.state.doc.content.size,
  });
}

function pasteText(editor, value) {
  const event = new dom.window.Event("paste", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "clipboardData", {
    value: {
      types: ["text/plain"],
      getData: (type) => (type === "text/plain" ? value : ""),
    },
  });
  editor.view.dom.dispatchEvent(event);
}

function linkHrefs(editor) {
  const hrefs = [];
  editor.state.doc.descendants((node) => {
    for (const mark of node.marks) {
      if (mark.type.name === "link") hrefs.push(mark.attrs.href);
    }
  });
  return hrefs;
}

function nodeTypeNames(editor) {
  const names = [];
  editor.state.doc.descendants((node) => {
    names.push(node.type.name);
  });
  return names;
}

test("mixed paragraph and code-block selection is replaced, never part-linked", async () => {
  const editor = await mountComposerEditor();
  seedDocument(
    editor,
    paragraph(text("ordinary")),
    codeBlock("const value = 1;"),
  );
  selectAll(editor);

  pasteText(editor, "https://example.com");

  // The whole selection goes, exactly as it would for any non-link paste.
  assert.equal(editor.state.doc.textContent, "https://example.com");
  assert.ok(!nodeTypeNames(editor).includes("codeBlock"));
  // Crucially, "ordinary" is not left behind wearing a link mark.
  assert.ok(!editor.state.doc.textContent.includes("ordinary"));
});

test("mixed selection paste of a Buzz link becomes a chip, not a part-link", async () => {
  const editor = await mountComposerEditor();
  seedDocument(
    editor,
    paragraph(text("ordinary")),
    codeBlock("const value = 1;"),
  );
  selectAll(editor);

  pasteText(editor, CHANNEL_HREF);

  const names = nodeTypeNames(editor);
  assert.ok(names.includes("composerMessageLink"));
  assert.ok(!names.includes("codeBlock"));
  assert.ok(!editor.state.doc.textContent.includes("ordinary"));
  assert.deepEqual(linkHrefs(editor), []);
});

test("mixed plain and inline-code selection is replaced, never part-linked", async () => {
  const editor = await mountComposerEditor();
  seedDocument(
    editor,
    paragraph(text("plain "), text("inline", [{ type: "code" }])),
  );
  selectAll(editor);

  pasteText(editor, "https://example.com");

  assert.ok(!editor.state.doc.textContent.includes("plain "));
  assert.equal(editor.state.doc.textContent.trim(), "https://example.com");
});

test("fully markable selection keeps its label and gains the link", async () => {
  const editor = await mountComposerEditor();
  seedDocument(editor, paragraph(text("read this")));
  selectAll(editor);

  pasteText(editor, "https://example.com");

  assert.equal(editor.state.doc.textContent, "read this");
  assert.deepEqual(linkHrefs(editor), ["https://example.com"]);
});

test("fully markable selection keeps its label for linkify-only URL shapes", async () => {
  for (const [pasted, expectedHref] of [
    ["www.example.com", "http://www.example.com"],
    ["foo@example.com", "mailto:foo@example.com"],
  ]) {
    const editor = await mountComposerEditor();
    seedDocument(editor, paragraph(text("read this")));
    selectAll(editor);

    pasteText(editor, pasted);

    assert.equal(editor.state.doc.textContent, "read this");
    assert.deepEqual(linkHrefs(editor), [expectedHref]);
  }
});
