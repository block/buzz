import assert from "node:assert/strict";
import test from "node:test";

import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Markdown as TiptapMarkdown } from "tiptap-markdown";

import { ComposerText } from "./composerTextNode.ts";

function createEditor(content) {
  return new Editor({
    element: null,
    extensions: [
      StarterKit.configure({
        heading: false,
        link: false,
        text: false,
        trailingNode: false,
      }),
      ComposerText,
      TiptapMarkdown.configure({
        html: false,
        breaks: true,
      }),
    ],
    content,
  });
}

function serializeComposerMarkdown(editor) {
  // Mirror getMarkdownFromEditor: the composer intentionally removes
  // prosemirror-markdown's backslash escapes for user-authored Markdown.
  let markdown = editor.storage.markdown.getMarkdown();
  markdown = markdown.replace(/\\\n/g, "\n");
  return markdown.replace(/\\([`*\\~[\]_])/g, "$1");
}

test("inline-code Markdown preserves angle brackets in Markdown serialization", () => {
  const editor = createEditor({
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            text: "`<repo root>/website`",
          },
        ],
      },
    ],
  });

  try {
    assert.equal(serializeComposerMarkdown(editor), "`<repo root>/website`");
  } finally {
    editor.destroy();
  }
});

test("typed HTML entities remain literal text", () => {
  const editor = createEditor({
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [{ type: "text", text: "literal &lt;repo&gt;" }],
      },
    ],
  });

  try {
    assert.equal(serializeComposerMarkdown(editor), "literal &lt;repo&gt;");
  } finally {
    editor.destroy();
  }
});

test("Markdown punctuation and inline-code delimiters remain unchanged", () => {
  const editor = createEditor({
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            text: "**bold** and `inline` [brackets] ~tilde~",
          },
        ],
      },
    ],
  });

  try {
    assert.equal(
      serializeComposerMarkdown(editor),
      "**bold** and `inline` [brackets] ~tilde~",
    );
  } finally {
    editor.destroy();
  }
});
