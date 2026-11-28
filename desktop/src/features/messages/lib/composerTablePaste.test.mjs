/**
 * Regression coverage for pasting a Markdown table into the composer (#8530).
 *
 * The composer schema has no table nodes, and Enter sends, so a table can
 * only live in the composer as its source lines joined by hard breaks — the
 * same shape Shift+Enter builds. Both paste paths used to break that:
 *
 * - Ctrl+V parsed the text as Markdown, markdown-it turned it into a
 *   `<table>`, and the schema kept only the cell text, run together.
 * - Ctrl+Shift+V (`view.pasteText`) made one paragraph per line, which sends
 *   as rows separated by blank lines.
 *
 * These tests mount the production editor from `useRichTextEditor` and check
 * the Markdown the send path reads.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const TABLE = [
  "| Name | Role | Status |",
  "| --- | --- | --- |",
  "| Alice | Engineer | Active |",
  "| Bob | Designer | Away |",
].join("\n");

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

/** Mounts the production composer editor and returns the hook result. */
async function mountComposer() {
  const React = await import("react");
  const { act, render, waitFor } = await import("@testing-library/react");
  const { EditorContent } = await import("@tiptap/react");
  const { useRichTextEditor } = await import("./useRichTextEditor.ts");

  let composer = null;
  function Harness() {
    composer = useRichTextEditor({});
    return composer.editor
      ? React.createElement(EditorContent, { editor: composer.editor })
      : null;
  }

  await act(async () => {
    render(React.createElement(Harness));
  });
  await waitFor(() =>
    assert.ok(
      composer?.editor?.isInitialized,
      "composer editor never emitted `create`",
    ),
  );
  return composer;
}

/** A Ctrl+V of text copied from a plain-text editor: no HTML flavor. */
function pasteFromClipboard(editor, value) {
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

function topLevelTypes(editor) {
  return editor.state.doc.content.content.map((node) => node.type.name);
}

test("Ctrl+V keeps a Markdown table's rows and cells", async () => {
  const composer = await mountComposer();

  pasteFromClipboard(composer.editor, TABLE);

  assert.equal(composer.getMarkdown(), TABLE);
});

test("Ctrl+V keeps a Windows (CRLF) table's rows on single newlines", async () => {
  const composer = await mountComposer();

  pasteFromClipboard(composer.editor, TABLE.replaceAll("\n", "\r\n"));

  assert.equal(composer.getMarkdown(), TABLE);
});

test("Ctrl+V still parses other Markdown", async () => {
  const composer = await mountComposer();

  pasteFromClipboard(composer.editor, "**bold** text");

  assert.equal(composer.editor.state.doc.textContent, "bold text");
  assert.equal(composer.getMarkdown(), "**bold** text");
});

test("Ctrl+Shift+V keeps single newlines between table rows", async () => {
  const composer = await mountComposer();

  // The same call the Mod+Shift+V handler makes with the clipboard text.
  composer.editor.view.pasteText(TABLE);

  assert.deepEqual(topLevelTypes(composer.editor), ["paragraph"]);
  assert.equal(composer.getMarkdown(), TABLE);
});

test("Ctrl+Shift+V keeps blank lines as paragraph breaks", async () => {
  const composer = await mountComposer();

  composer.editor.view.pasteText("first\nstill first\n\nsecond");

  assert.deepEqual(topLevelTypes(composer.editor), ["paragraph", "paragraph"]);
  assert.equal(composer.getMarkdown(), "first\nstill first\n\nsecond");
});

test("loading a message that holds a table keeps its rows", async () => {
  const composer = await mountComposer();

  // Editing a sent message loads its Markdown through the same parser.
  composer.setContent(TABLE);

  assert.equal(composer.getMarkdown(), TABLE);
});
