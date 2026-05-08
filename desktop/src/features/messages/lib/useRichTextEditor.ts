import * as React from "react";

import { Markdown as TiptapMarkdown } from "tiptap-markdown";
import { useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Link from "@tiptap/extension-link";
import { Extension } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";

import {
  MentionHighlightExtension,
  mentionHighlightKey,
} from "./mentionHighlightExtension";

export type RichTextEditorOptions = {
  placeholder?: string;
  onUpdate?: (info: { markdown: string; text: string }) => void;
  editable?: boolean;
  mentionNames?: string[];
  channelNames?: string[];
  /** Called on plain Enter (submit). Handled inside Tiptap's extension system
   *  so it fires *before* ProseMirror's default splitBlock behaviour. */
  onSubmit?: () => void;
  /** When true, plain Enter is passed through (e.g. to select an autocomplete item). */
  isAutocompleteOpen?: React.RefObject<boolean>;
};

/**
 * Creates and manages a Tiptap editor configured for Markdown output.
 *
 * The editor uses StarterKit (bold, italic, strike, code, blockquote, lists,
 * headings, code blocks, hard breaks) plus Link and the tiptap-markdown
 * extension for serialisation.
 *
 * `getMarkdown()` returns the current document as a Markdown string.
 */
export function useRichTextEditor({
  placeholder,
  onUpdate,
  editable = true,
  mentionNames,
  channelNames,
  onSubmit,
  isAutocompleteOpen,
}: RichTextEditorOptions) {
  const onUpdateRef = React.useRef(onUpdate);
  onUpdateRef.current = onUpdate;

  const onSubmitRef = React.useRef(onSubmit);
  onSubmitRef.current = onSubmit;

  const placeholderRef = React.useRef(placeholder);
  placeholderRef.current = placeholder;

  const editor = useEditor(
    {
      extensions: [
        StarterKit.configure({
          // Use hard breaks (Shift+Enter) — Enter submits the message.
          hardBreak: {
            keepMarks: true,
          },
          // Disable heading input rules — in a chat composer, typing "# "
          // should keep the literal "#", not convert to a heading node.
          // Users type #channel-name and the "#" would get eaten otherwise.
          heading: false,
          // Disable the trailing-node plugin — it forces an empty paragraph
          // after block nodes (lists, blockquotes, code blocks) which creates
          // a phantom empty line in the compact message composer.
          trailingNode: false,
          // Disable StarterKit's built-in Link — we configure it separately
          // below with custom options (autolink, openOnClick, etc.).
          link: false,
        }),
        // Shift+Enter inside lists/blockquotes: split the node instead of
        // inserting a hard break so continuation lines keep their formatting.
        Extension.create({
          name: "smartShiftEnter",
          addKeyboardShortcuts() {
            // Exit a list by removing the empty last item and inserting a
            // paragraph after the list. Works for both single-item and
            // multi-item lists.
            const exitListIfEmptyLast = (ed: typeof this.editor): boolean => {
              if (!ed.isActive("listItem")) return false;
              const { $from } = ed.state.selection;

              // Walk up to find the listItem node (handles nested structures).
              let listItemDepth = -1;
              for (let d = $from.depth; d >= 1; d--) {
                if ($from.node(d).type.name === "listItem") {
                  listItemDepth = d;
                  break;
                }
              }
              if (listItemDepth < 1) return false;

              const listItem = $from.node(listItemDepth);
              const isEmpty =
                listItem.childCount === 1 &&
                listItem.firstChild?.textContent === "";
              if (!isEmpty) return false;

              // Only trigger on the last item in the list.
              const listDepth = listItemDepth - 1;
              const list = $from.node(listDepth);
              const itemIndex = $from.index(listDepth);
              if (itemIndex !== list.childCount - 1) return false;

              const { tr, schema } = ed.state;
              if (list.childCount === 1) {
                // Only item → replace the entire list with an empty paragraph.
                const listStart = $from.before(listDepth);
                const listEnd = $from.after(listDepth);
                const para = schema.nodes.paragraph.create();
                tr.replaceWith(listStart, listEnd, para);
                tr.setSelection(
                  TextSelection.near(tr.doc.resolve(listStart + 1)),
                );
              } else {
                // Multiple items → delete the empty item, insert paragraph
                // after the list, and move cursor there.
                const itemStart = $from.before(listItemDepth);
                const itemEnd = $from.after(listItemDepth);
                tr.delete(itemStart, itemEnd);
                const listEnd = tr.mapping.map($from.after(listDepth));
                const para = schema.nodes.paragraph.create();
                tr.insert(listEnd, para);
                tr.setSelection(
                  TextSelection.near(tr.doc.resolve(listEnd + 1)),
                );
              }
              ed.view.dispatch(tr);
              return true;
            };

            return {
              "Shift-Enter": ({ editor: ed }) => {
                // Empty last list item → exit list to paragraph below.
                if (exitListIfEmptyLast(ed)) return true;
                // Non-empty or non-last list item → split.
                if (ed.isActive("listItem")) {
                  return ed.commands.splitListItem("listItem");
                }
                if (ed.isActive("blockquote")) {
                  // Empty blockquote paragraph → exit the blockquote.
                  const { $from } = ed.state.selection;
                  if ($from.parent.textContent === "") {
                    return ed.commands.lift("blockquote");
                  }
                  // Non-empty → split the paragraph within the blockquote.
                  return ed.chain().splitBlock().focus().run();
                }
                // Default: hard break (StarterKit handles it).
                return false;
              },
              ArrowDown: ({ editor: ed }) => {
                // Empty last list item + Down → exit list to paragraph below.
                return exitListIfEmptyLast(ed);
              },
            };
          },
        }),
        // Plain Enter → submit the message. This runs inside ProseMirror's
        // keymap pipeline so it fires *before* the default splitBlock command,
        // preventing the phantom paragraph-split that caused \n\n in messages.
        Extension.create({
          name: "submitOnEnter",
          addKeyboardShortcuts() {
            return {
              Enter: () => {
                // Let autocomplete dropdowns consume Enter first.
                if (isAutocompleteOpen?.current) return false;
                // No submit callback → fall through to default behaviour.
                if (!onSubmitRef.current) return false;
                onSubmitRef.current();
                return true; // prevents splitBlock
              },
            };
          },
        }),
        MentionHighlightExtension,
        Placeholder.configure({
          placeholder: () => placeholderRef.current ?? "Write a message…",
        }),
        Link.extend({
          inclusive() {
            return false;
          },
        }).configure({
          openOnClick: false,
          autolink: true,
          linkOnPaste: true,
          HTMLAttributes: {
            class: "text-primary underline underline-offset-4 cursor-pointer",
          },
        }),
        TiptapMarkdown.configure({
          html: false,
          transformPastedText: true,
          transformCopiedText: true,
          breaks: true,
        }),
      ],
      editorProps: {
        attributes: {
          class:
            "min-h-0 resize-none overflow-y-hidden border-0 bg-transparent px-0 py-0 text-sm leading-6 md:leading-6 shadow-none focus-visible:ring-0 caret-foreground outline-none prose-sm max-w-none",
          "data-testid": "message-input",
        },
      },
      onUpdate: ({ editor: ed }) => {
        const markdown = getMarkdownFromEditor(ed);
        const text = ed.state.doc.textContent;
        onUpdateRef.current?.({ markdown, text });
      },
    },
    [],
  );

  // Toggle editable without destroying the editor instance.
  React.useEffect(() => {
    if (editor && editor.isEditable !== editable) {
      editor.setEditable(editable);
    }
  }, [editor, editable]);

  // Update placeholder text without recreating the editor.
  // biome-ignore lint/correctness/useExhaustiveDependencies: placeholder triggers the ref update
  React.useEffect(() => {
    if (!editor) return;
    // Force ProseMirror to re-run decoration plugins so the Placeholder
    // extension picks up the new text from placeholderRef.
    editor.view.dispatch(editor.state.tr);
  }, [editor, placeholder]);

  // Keep mention/channel-highlight decorations in sync with known names.
  // NOTE: We use `editor.storage.mentionHighlight` (the mutable storage object
  // shared with the ProseMirror plugin closure) rather than finding the
  // extension instance via extensionManager — the instance's `.storage` getter
  // returns a fresh spread-copy on every access, so mutations are silently lost.
  React.useEffect(() => {
    if (!editor) return;
    // biome-ignore lint/suspicious/noExplicitAny: TipTap's Storage type doesn't include dynamic extension keys
    const storage = (editor.storage as any).mentionHighlight as
      | { names: string[]; channelNames: string[] }
      | undefined;
    if (storage) {
      storage.names = mentionNames ?? [];
      storage.channelNames = channelNames ?? [];
      // Force the plugin to re-decorate by dispatching a metadata transaction.
      const { tr } = editor.state;
      editor.view.dispatch(tr.setMeta(mentionHighlightKey, true));
    }
  }, [editor, mentionNames, channelNames]);

  const getMarkdown = React.useCallback((): string => {
    if (!editor) return "";
    return getMarkdownFromEditor(editor);
  }, [editor]);

  const isEmpty = React.useCallback((): boolean => {
    if (!editor) return true;
    return editor.isEmpty;
  }, [editor]);

  const clearContent = React.useCallback(() => {
    editor?.commands.clearContent(true);
  }, [editor]);

  const setContent = React.useCallback(
    (markdown: string) => {
      if (!editor) return;
      editor.commands.setContent(markdown);
    },
    [editor],
  );

  const focus = React.useCallback(() => {
    editor?.commands.focus("end");
  }, [editor]);

  /**
   * Replace editor content and append a trailing space that survives parsing.
   *
   * `setContent(markdown)` roundtrips through TipTap's markdown parser which
   * strips trailing whitespace from text nodes. TipTap's `insertContent(" ")`
   * also normalises it away. This method bypasses both by creating a raw
   * ProseMirror text node and inserting it via a direct transaction — the
   * only path that reliably preserves a literal trailing space.
   *
   * Used by mention and channel-link autocomplete insertion.
   */
  const setContentWithTrailingSpace = React.useCallback(
    (markdown: string) => {
      if (!editor) return;
      editor.commands.setContent(markdown);
      // Insert a literal space via a raw ProseMirror transaction so it
      // bypasses TipTap's content parser which strips trailing whitespace.
      const { tr, schema, doc } = editor.state;
      const endPos = doc.content.size - 1; // before the closing node token
      const spaceNode = schema.text(" ");
      tr.insert(endPos, spaceNode);
      // Place cursor after the inserted space.
      const cursorPos = endPos + spaceNode.nodeSize;
      tr.setSelection(TextSelection.create(tr.doc, cursorPos));
      editor.view.dispatch(tr);
      editor.view.focus();
    },
    [editor],
  );

  /**
   * Returns the plain-text content and an approximate cursor offset.
   * Used to bridge the existing useMentions / useChannelLinks hooks which
   * were designed for a plain <textarea>.
   */
  const getTextAndCursor = React.useCallback((): {
    text: string;
    cursor: number;
  } => {
    if (!editor) return { text: "", cursor: 0 };

    const { state } = editor;
    const text = state.doc.textContent;
    // Map ProseMirror position → plain-text offset.
    // Walk through text nodes and accumulate length until we pass the anchor.
    const anchor = state.selection.anchor;
    let offset = 0;
    let found = false;
    state.doc.descendants((node, pos) => {
      if (found) return false;
      if (node.isText) {
        const nodeEnd = pos + node.nodeSize;
        if (anchor <= nodeEnd) {
          offset += anchor - pos;
          found = true;
          return false;
        }
        offset += node.nodeSize;
      } else if (node.isBlock && pos > 0) {
        // Block boundaries add a newline in textContent
        // (but only between blocks, not at the very start)
        offset += 1;
      }
      return undefined;
    });
    if (!found) {
      offset = text.length;
    }

    return { text, cursor: offset };
  }, [editor]);

  return {
    editor,
    getMarkdown,
    isEmpty,
    clearContent,
    setContent,
    setContentWithTrailingSpace,
    focus,
    getTextAndCursor,
  };
}

function getMarkdownFromEditor(editor: Editor): string {
  // biome-ignore lint/suspicious/noExplicitAny: tiptap-markdown storage is untyped
  const storage = (editor.storage as any).markdown as
    | { getMarkdown?: () => string }
    | undefined;
  if (storage?.getMarkdown) {
    return storage.getMarkdown();
  }
  // Fallback: plain text
  return editor.state.doc.textContent;
}
