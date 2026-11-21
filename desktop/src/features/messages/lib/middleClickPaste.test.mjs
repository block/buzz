/**
 * Middle-click paste on Linux, driven through the production composer editor.
 *
 * WebKitGTK hands a middle-click `paste` event the Ctrl+C clipboard while its
 * native insertion carries the primary selection. jsdom has no native paste,
 * so these tests play WebKit's part: they dispatch the `paste` event with the
 * clipboard text, then write the primary selection into whatever element took
 * focus, as WebKit's default action would.
 */
import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const CLIPBOARD = "from the clipboard";
const PRIMARY = "from the selection";

class FakeDataTransfer {
  #data = new Map();
  get items() {
    return [];
  }
  get types() {
    return [...this.#data.keys()];
  }
  getData(type) {
    return this.#data.get(type) ?? "";
  }
  setData(type, value) {
    this.#data.set(type, value);
  }
}

before(() => {
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  // Middle-click paste is a Linux-only gesture.
  Object.defineProperty(dom.window.navigator, "platform", {
    configurable: true,
    value: "Linux x86_64",
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
  Object.assign(globalThis, {
    ClipboardEvent: dom.window.Event,
    CustomEvent: dom.window.CustomEvent,
    DataTransfer: FakeDataTransfer,
    DOMParser: dom.window.DOMParser,
    document: dom.window.document,
    Element: dom.window.Element,
    Event: dom.window.Event,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MouseEvent: dom.window.MouseEvent,
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

async function mountComposerEditor() {
  const React = await import("react");
  const { act, render, waitFor } = await import("@testing-library/react");
  const { EditorContent } = await import("@tiptap/react");
  const { useRichTextEditor } = await import("./useRichTextEditor.ts");

  let editor = null;
  function Harness() {
    const instance = useRichTextEditor({}).editor;
    editor = instance;
    return instance
      ? React.createElement(EditorContent, { editor: instance })
      : null;
  }

  await act(async () => {
    render(React.createElement(Harness));
  });
  await waitFor(() => assert.ok(editor?.isInitialized));
  editor.commands.setContent("hello world");
  // jsdom has no layout; pretend the click landed after "hello ".
  editor.view.posAtCoords = () => ({ pos: 7, inside: -1 });
  return editor;
}

function press(editor, button) {
  editor.view.dom.dispatchEvent(
    new dom.window.MouseEvent("mousedown", { bubbles: true, button }),
  );
}

/** Dispatches WebKit's `paste`, then performs its native insertion. */
function webkitPaste(editor) {
  const clipboardData = new FakeDataTransfer();
  clipboardData.setData("text/plain", CLIPBOARD);
  const event = new dom.window.Event("paste", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "clipboardData", { value: clipboardData });
  editor.view.dom.dispatchEvent(event);
  const focused = dom.window.document.activeElement;
  if (
    !event.defaultPrevented &&
    focused instanceof dom.window.HTMLTextAreaElement
  ) {
    focused.value += PRIMARY;
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 100));

test("middle-click pastes the primary selection at the click", async () => {
  const editor = await mountComposerEditor();
  press(editor, 1);
  webkitPaste(editor);
  await settle();
  assert.equal(editor.getText(), `hello ${PRIMARY}world`);
  assert.equal(dom.window.document.querySelector("textarea"), null);
});

test("keyboard paste still uses the clipboard", async () => {
  const editor = await mountComposerEditor();
  editor.commands.setTextSelection(7);
  webkitPaste(editor);
  await settle();
  assert.equal(editor.getText(), `hello ${CLIPBOARD}world`);
});

test("a left click does not redirect the next paste", async () => {
  const editor = await mountComposerEditor();
  press(editor, 0);
  editor.commands.setTextSelection(7);
  webkitPaste(editor);
  await settle();
  assert.equal(editor.getText(), `hello ${CLIPBOARD}world`);
});

test("a middle click redirects only the paste that follows it", async () => {
  const editor = await mountComposerEditor();
  press(editor, 1);
  webkitPaste(editor);
  await settle();
  editor.commands.setTextSelection(1);
  webkitPaste(editor);
  await settle();
  assert.equal(editor.getText(), `${CLIPBOARD}hello ${PRIMARY}world`);
});

test("other platforms leave middle clicks alone", async () => {
  Object.defineProperty(dom.window.navigator, "platform", {
    configurable: true,
    value: "MacIntel",
  });
  try {
    const editor = await mountComposerEditor();
    press(editor, 1);
    editor.commands.setTextSelection(7);
    webkitPaste(editor);
    await settle();
    assert.equal(editor.getText(), `hello ${CLIPBOARD}world`);
  } finally {
    Object.defineProperty(dom.window.navigator, "platform", {
      configurable: true,
      value: "Linux x86_64",
    });
  }
});
