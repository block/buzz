import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import StarterKit from "@tiptap/starter-kit";
import { JSDOM } from "jsdom";

import { handleMentionBoundaryBeforeInput } from "./mentionBoundaryBeforeInput.ts";
import {
  MentionHighlightExtension,
  mentionHighlightKey,
} from "./mentionHighlightExtension.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});
const editors = new Set();

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    DocumentFragment: dom.window.DocumentFragment,
    DOMParser: dom.window.DOMParser,
    Element: dom.window.Element,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    getSelection: dom.window.getSelection.bind(dom.window),
    HTMLElement: dom.window.HTMLElement,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
    window: dom.window,
  });
});

afterEach(() => {
  for (const editor of editors) editor.destroy();
  editors.clear();
});

after(() => dom.window.close());

function paragraphContent(text) {
  return text ? [{ type: "text", text }] : [];
}

function createView(text, options = {}) {
  const editor = new Editor({
    content: {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: options.content ?? paragraphContent(text),
        },
      ],
    },
    element: document.createElement("div"),
    extensions: [
      StarterKit.configure({
        heading: false,
        link: false,
        trailingNode: false,
      }),
      MentionHighlightExtension,
    ],
  });
  editors.add(editor);

  const storage = editor.storage.mentionHighlight;
  storage.names = [];
  storage.agentNames = options.decorated ? ["Reinhold"] : [];
  storage.channelNames = [];
  editor.view.dispatch(editor.state.tr.setMeta(mentionHighlightKey, true));

  const doc = editor.state.doc;
  const textLength = doc.textContent.length;
  const from = options.from ?? 1 + textLength;
  const to = options.to ?? from;
  editor.view.dispatch(
    editor.state.tr.setSelection(TextSelection.create(doc, from, to)),
  );

  let dispatchCount = 0;
  const originalDispatch = editor.view.dispatch.bind(editor.view);
  editor.view.dispatch = (transaction) => {
    dispatchCount += 1;
    originalDispatch(transaction);
  };

  return {
    dom: editor.view.dom,
    domAtPos(position) {
      return editor.view.domAtPos(position);
    },
    get state() {
      return editor.state;
    },
    composing: options.composing ?? false,
    dispatch(transaction) {
      editor.view.dispatch(transaction);
    },
    get dispatchCount() {
      return dispatchCount;
    },
  };
}

function createBeforeInput(overrides = {}) {
  let prevented = false;
  return {
    inputType: "insertText",
    data: "t",
    isComposing: false,
    preventDefault() {
      prevented = true;
    },
    get prevented() {
      return prevented;
    },
    ...overrides,
  };
}

test("inserts the first character after a highlighted agent mention through ProseMirror", () => {
  const view = createView("@Reinhold ", { decorated: true });
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), true);
  assert.equal(event.prevented, true);
  assert.equal(view.dispatchCount, 1);
  assert.equal(view.state.doc.textContent, "@Reinhold t");
  assert.equal(
    view.state.doc.textContent.codePointAt("@Reinhold".length),
    0x20,
  );
  assert.equal(view.state.selection.from, 1 + "@Reinhold t".length);
});

test("maps the caret after the separator outside the agent chip DOM", () => {
  const view = createView("@Reinhold ", { decorated: true });
  const label = view.dom.querySelector(".agent-mention-highlight");
  assert.ok(label);

  const separator = label.nextSibling;
  assert.equal(separator?.nodeType, Node.TEXT_NODE);
  assert.equal(separator?.textContent, " ");
  assert.equal(view.state.selection.from, 1 + "@Reinhold ".length);

  const mappedCaret = view.domAtPos(view.state.selection.from);
  assert.ok(
    (mappedCaret.node === separator && mappedCaret.offset === 1) ||
      (mappedCaret.node === separator?.parentNode &&
        mappedCaret.offset ===
          Array.prototype.indexOf.call(
            separator.parentNode.childNodes,
            separator,
          ) +
            1),
  );
});

test("leaves ordinary typing outside an agent-mention boundary to the browser", () => {
  const view = createView("ordinary ");
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), false);
  assert.equal(event.prevented, false);
  assert.equal(view.dispatchCount, 0);
});

test("requires a collapsed selection after a U+0020 separator", () => {
  const selectedView = createView("@Reinhold ", { from: 1, to: 2 });
  const selectedEvent = createBeforeInput();
  assert.equal(
    handleMentionBoundaryBeforeInput(selectedView, selectedEvent),
    false,
  );

  const noSpaceView = createView("@Reinhold");
  const noSpaceEvent = createBeforeInput();
  assert.equal(
    handleMentionBoundaryBeforeInput(noSpaceView, noSpaceEvent),
    false,
  );
});

test("does not intercept an unhighlighted mention", () => {
  const view = createView("@Reinhold ");
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), false);
  assert.equal(event.prevented, false);
  assert.equal(view.dispatchCount, 0);
});

test("does not infer an agent mention from marked text without a decoration", () => {
  const view = createView("@Reinhold ", {
    content: [
      { type: "text", marks: [{ type: "bold" }], text: "@Reinhold" },
      { type: "text", text: " " },
    ],
  });
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), false);
  assert.equal(event.prevented, false);
  assert.equal(view.dispatchCount, 0);
});

test("accepts marked text when the exact mention range is decorated", () => {
  const view = createView("@Reinhold ", {
    decorated: true,
    content: [
      { type: "text", marks: [{ type: "bold" }], text: "@Reinhold" },
      { type: "text", text: " " },
    ],
  });
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), true);
  assert.equal(event.prevented, true);
  assert.equal(view.dispatchCount, 1);
  assert.equal(view.state.doc.textContent, "@Reinhold t");
});

test("does not infer an agent mention across split text nodes", () => {
  const view = createView("@Reinhold ", {
    decorated: true,
    content: [
      { type: "text", marks: [{ type: "bold" }], text: "@Rein" },
      { type: "text", text: "hold " },
    ],
  });
  const event = createBeforeInput();

  assert.equal(handleMentionBoundaryBeforeInput(view, event), false);
  assert.equal(event.prevented, false);
  assert.equal(view.dispatchCount, 0);
});

test("does not intercept paste or replacement input", () => {
  for (const inputType of ["insertFromPaste", "insertReplacementText"]) {
    const view = createView("@Reinhold ", { decorated: true });
    const event = createBeforeInput({ inputType });
    assert.equal(handleMentionBoundaryBeforeInput(view, event), false);
    assert.equal(event.prevented, false);
    assert.equal(view.dispatchCount, 0);
  }
});

test("does not intercept composition or IME input", () => {
  const composingEventView = createView("@Reinhold ", { decorated: true });
  const composingEvent = createBeforeInput({ isComposing: true });
  assert.equal(
    handleMentionBoundaryBeforeInput(composingEventView, composingEvent),
    false,
  );

  const composingView = createView("@Reinhold ", {
    composing: true,
    decorated: true,
  });
  const commitEvent = createBeforeInput();
  assert.equal(
    handleMentionBoundaryBeforeInput(composingView, commitEvent),
    false,
  );
});
