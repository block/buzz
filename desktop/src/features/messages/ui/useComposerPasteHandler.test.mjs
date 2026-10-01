import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Event: dom.window.Event,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    KeyboardEvent: dom.window.KeyboardEvent,
    window: dom.window,
  });
});

after(() => dom.window.close());

/** Minimal stand-in for the Tiptap editor the hook configures. */
function fakeEditor() {
  const dom_ = dom.window.document.createElement("div");
  return {
    options: { editorProps: {} },
    setOptions(next) {
      Object.assign(this.options, next);
    },
    view: { dom: dom_ },
  };
}

function pasteEvent(clipboardData) {
  let defaultPrevented = false;
  return {
    clipboardData,
    preventDefault() {
      defaultPrevented = true;
    },
    get defaultPrevented() {
      return defaultPrevented;
    },
  };
}

function emptyClipboard() {
  return {
    files: [],
    getData: () => "",
    items: [],
    types: [],
  };
}

async function mountHandler() {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useComposerPasteHandler } = await import(
    "./useComposerPasteHandler.ts"
  );
  const editor = fakeEditor();
  const deferredCalls = [];
  const directCalls = [];
  renderHook(() =>
    useComposerPasteHandler({
      editor,
      scrollToBottom: () => {},
      setPendingImeta: () => {},
      uploadDeferredFile: async (readFile) => {
        deferredCalls.push(readFile);
      },
      uploadFile: async (file) => {
        directCalls.push(file);
      },
    }),
  );
  return { act, cleanup, deferredCalls, directCalls, editor };
}

test("Ctrl+V with an empty DataTransfer routes to the native clipboard read", async () => {
  const { act, cleanup, deferredCalls, editor } = await mountHandler();

  editor.view.dom.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { ctrlKey: true, key: "v" }),
  );
  const event = pasteEvent(emptyClipboard());
  const handled = editor.options.editorProps.handlePaste({}, event);

  assert.equal(handled, true, "the handler must own the paste");
  assert.equal(event.defaultPrevented, true);
  assert.equal(deferredCalls.length, 1, "one deferred upload must be reserved");

  await act(async () => {});
  cleanup();
});

test("a middle-click paste with the same empty DataTransfer is not hijacked", async () => {
  const { act, cleanup, deferredCalls, editor } = await mountHandler();

  // No preceding Ctrl/Cmd+V: WebKitGTK reports the same empty DataTransfer
  // for a middle-click paste of PRIMARY, but arboard reads CLIPBOARD, so
  // firing here would upload an unrelated image.
  const event = pasteEvent(emptyClipboard());
  const handled = editor.options.editorProps.handlePaste({}, event);

  assert.equal(handled, false, "the paste must fall through untouched");
  assert.equal(event.defaultPrevented, false);
  assert.equal(deferredCalls.length, 0, "no clipboard read may be issued");

  await act(async () => {});
  cleanup();
});

test("a text paste still falls through after a Ctrl+V", async () => {
  const { act, cleanup, deferredCalls, editor } = await mountHandler();

  editor.view.dom.dispatchEvent(
    new dom.window.KeyboardEvent("keydown", { ctrlKey: true, key: "v" }),
  );
  const event = pasteEvent({
    files: [],
    getData: (type) => (type === "text/plain" ? "hello" : ""),
    items: [{ kind: "string" }],
    types: ["text/plain"],
  });
  const handled = editor.options.editorProps.handlePaste({}, event);

  assert.equal(handled, false);
  assert.equal(deferredCalls.length, 0);

  await act(async () => {});
  cleanup();
});
