import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { JSDOM } from "jsdom";
import { Markdown } from "tiptap-markdown";

import { setEditorMarkdownPreservingTrailingWhitespace } from "./setEditorMarkdownPreservingTrailingWhitespace.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

/** @type {Editor | null} */
let editor = null;

function plainText() {
  assert.ok(editor);
  return editor.state.doc.textBetween(
    0,
    editor.state.doc.content.size,
    "\n",
    "\n",
  );
}

before(() => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    DOMParser: dom.window.DOMParser,
    Node: dom.window.Node,
    DocumentFragment: dom.window.DocumentFragment,
    HTMLElement: dom.window.HTMLElement,
    Element: dom.window.Element,
    MutationObserver: dom.window.MutationObserver,
    getSelection: dom.window.getSelection.bind(dom.window),
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    requestAnimationFrame: dom.window.requestAnimationFrame.bind(dom.window),
  });

  editor = new Editor({
    element: document.createElement("div"),
    extensions: [
      StarterKit.configure({
        trailingNode: false,
        heading: false,
        link: false,
      }),
      Markdown.configure({
        html: false,
        transformPastedText: true,
        transformCopiedText: true,
      }),
    ],
    content: "",
  });
});

after(() => {
  editor?.destroy();
  editor = null;
});

test("markdown setContent alone strips the trailing space (#4979 repro)", () => {
  assert.ok(editor);
  editor.commands.setContent("@Pearl ");
  assert.equal(plainText(), "@Pearl");
});

test("helper keeps a trailing space after a mention restore", () => {
  assert.ok(editor);
  setEditorMarkdownPreservingTrailingWhitespace(editor, "@Pearl ", {
    emitUpdate: false,
    focusEnd: true,
  });
  assert.equal(plainText(), "@Pearl ");
  assert.equal(editor.state.selection.from, editor.state.doc.content.size - 1);
});

test("helper keeps trailing space after multiple mentions", () => {
  assert.ok(editor);
  setEditorMarkdownPreservingTrailingWhitespace(editor, "@Vogue @Morgarita ", {
    emitUpdate: false,
    focusEnd: true,
  });
  assert.equal(plainText(), "@Vogue @Morgarita ");
});

test("helper still parses markdown marks in the body", () => {
  assert.ok(editor);
  setEditorMarkdownPreservingTrailingWhitespace(editor, "**bold** ", {
    emitUpdate: false,
    focusEnd: true,
  });
  assert.equal(plainText(), "bold ");
  let sawBold = false;
  editor.state.doc.descendants((node) => {
    if (node.isText && node.marks.some((mark) => mark.type.name === "bold")) {
      sawBold = true;
    }
  });
  assert.equal(sawBold, true);
});

test("helper is a no-op for content without trailing whitespace", () => {
  assert.ok(editor);
  setEditorMarkdownPreservingTrailingWhitespace(editor, "@Pearl", {
    emitUpdate: false,
    focusEnd: true,
  });
  assert.equal(plainText(), "@Pearl");
});

test("emitUpdate:false suppresses onUpdate for the re-attached space", () => {
  assert.ok(editor);
  let updateCount = 0;
  const onUpdate = () => {
    updateCount += 1;
  };
  editor.on("update", onUpdate);
  try {
    setEditorMarkdownPreservingTrailingWhitespace(editor, "@Pearl ", {
      emitUpdate: false,
      focusEnd: true,
    });
    assert.equal(plainText(), "@Pearl ");
    assert.equal(updateCount, 0);
  } finally {
    editor.off("update", onUpdate);
  }
});

test("emitUpdate:true still notifies observers after restore", () => {
  assert.ok(editor);
  let updateCount = 0;
  const onUpdate = () => {
    updateCount += 1;
  };
  editor.on("update", onUpdate);
  try {
    setEditorMarkdownPreservingTrailingWhitespace(editor, "@Pearl ");
    assert.equal(plainText(), "@Pearl ");
    assert.ok(updateCount >= 1);
  } finally {
    editor.off("update", onUpdate);
  }
});
