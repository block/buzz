import * as React from "react";

import { Markdown as TiptapMarkdown } from "tiptap-markdown";
import { useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Link from "@tiptap/extension-link";
import { Extension, type KeyboardShortcutCommand } from "@tiptap/core";
import { Selection, TextSelection } from "@tiptap/pm/state";

import { isMacPlatform } from "@/shared/lib/platform";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

import {
  MentionHighlightExtension,
  mentionHighlightKey,
} from "./mentionHighlightExtension";
import { CUSTOM_EMOJI_NODE_NAME } from "./customEmojiNode";
import { useComposerCustomEmoji } from "./useComposerCustomEmoji";
import { buildPlainTextProjection } from "./plainTextProjection";
import {
  CodeBlockAfterHardBreak,
  handleCodeFenceEnter,
  insertNewlineInCodeBlock,
} from "./codeBlockExtensions";

/**
 * Plain-text edit descriptor returned by autocomplete hooks
 * (mentions / channel links / emoji). Offsets are in plain-text space —
 * see `buildPlainTextProjection`.
 */
export type AutocompleteEdit = {
  replaceFromOffset: number;
  replaceToOffset: number;
  insertText: string;
  /**
   * When set, the replaced range becomes a CustomEmojiNode for this
   * shortcode (followed by `insertText`, which carries the trailing space)
   * instead of literal `:shortcode:` text. Lets the emoji autocomplete
   * insert the same selectable/copyable atom the input rule produces when
   * typing — input rules don't fire on programmatic inserts.
   */
  customEmojiShortcode?: string;
};

export type RichTextEditorOptions = {
  placeholder?: string;
  onUpdate?: (info: { markdown: string; text: string }) => void;
  editable?: boolean;
  mentionNames?: string[];
  agentMentionNames?: string[];
  channelNames?: string[];
  /** Known custom-emoji set; used to render `:shortcode:` inline as images. */
  customEmoji?: CustomEmoji[];
  /** Called on plain Enter (submit). Handled inside Tiptap's extension system
   *  so it fires *before* ProseMirror's default splitBlock behaviour. */
  onSubmit?: () => void;
  /**
   * Called on ArrowUp in an empty composer (Slack parity: edit your last
   * message). Handled inside ProseMirror's `editorProps.handleKeyDown` — the
   * raw DOM keydown hook that runs before any command/caret logic — so it
   * fires deterministically even immediately after a send while the editor
   * still holds DOM focus (where the keymap plugin and a wrapper-level
   * `onKeyDown` both fail to see the event because the WebView's
   * vertical-arrow handling consumes it first). The owner should locate the
   * most recent message authored by the current user within this composer's
   * scope and enter edit mode. Return `true` if a target was found and edit
   * mode was entered, so the keystroke is swallowed; return `false` to let
   * ArrowUp fall through to normal caret movement.
   */
  onEditLastOwnMessage?: () => boolean;
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
  agentMentionNames,
  channelNames,
  customEmoji,
  onSubmit,
  onEditLastOwnMessage,
  isAutocompleteOpen,
}: RichTextEditorOptions) {
  const onUpdateRef = React.useRef(onUpdate);
  onUpdateRef.current = onUpdate;

  const onSubmitRef = React.useRef(onSubmit);
  onSubmitRef.current = onSubmit;

  const onEditLastOwnMessageRef = React.useRef(onEditLastOwnMessage);
  onEditLastOwnMessageRef.current = onEditLastOwnMessage;

  const placeholderRef = React.useRef(placeholder);
  placeholderRef.current = placeholder;

  // Custom-emoji atom node wiring (config + src re-resolve). Kept in a sibling
  // hook so this file stays focused on generic editor setup.
  const customEmojiWiring = useComposerCustomEmoji(customEmoji);

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
        // macOS text fields traditionally support a small set of Emacs-style
        // Control shortcuts. ProseMirror already handles Ctrl-A/E/H/D on macOS;
        // these fill in the common movement and kill-line gaps for the composer.
        Extension.create({
          name: "macEmacsTextShortcuts",
          addKeyboardShortcuts() {
            const shortcuts: Record<string, KeyboardShortcutCommand> = {};
            if (!isMacPlatform()) {
              return shortcuts;
            }

            return {
              "Ctrl-b": ({ editor: ed }) => {
                const { empty, from } = ed.state.selection;
                if (!empty || from <= 0) return false;
                return ed.commands.setTextSelection(from - 1);
              },
              "Ctrl-f": ({ editor: ed }) => {
                const { empty, from } = ed.state.selection;
                if (!empty || from >= ed.state.doc.content.size) return false;
                return ed.commands.setTextSelection(from + 1);
              },
              "Ctrl-k": ({ editor: ed }) => {
                const { state, view } = ed;
                const { $from, empty, from, to } = state.selection;

                if (!empty) {
                  return ed.commands.deleteSelection();
                }

                const blockEnd = $from.end();
                if (from < blockEnd) {
                  return ed.commands.deleteRange({ from, to: blockEnd });
                }

                const nextSelection = Selection.findFrom(
                  state.doc.resolve(to),
                  1,
                  true,
                );
                if (!nextSelection) return false;

                const transaction = state.tr.delete(to, nextSelection.from);
                view.dispatch(transaction.scrollIntoView());
                return true;
              },
            };
          },
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
                if (ed.isActive("codeBlock")) {
                  return insertNewlineInCodeBlock(ed);
                }
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
              Enter: ({ editor: ed }) => {
                if (isAutocompleteOpen?.current) return false;
                if (!onSubmitRef.current) return false;

                const fenceResult = handleCodeFenceEnter(ed);
                if (fenceResult !== undefined) return fenceResult;

                onSubmitRef.current();
                return true;
              },
            };
          },
        }),
        CodeBlockAfterHardBreak,
        MentionHighlightExtension,
        customEmojiWiring.extension,
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
          // Allow Buzz message links through TipTap's URL sanitiser.
          // http(s) and mailto are accepted by default; non-listed protocols are
          // stripped on paste/typed input.
          protocols: ["buzz"],
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
          autocapitalize: "none",
          autocorrect: "off",
          class:
            "min-h-0 resize-none overflow-y-hidden border-0 bg-transparent px-0 py-0 text-sm leading-6 md:leading-6 shadow-none focus-visible:ring-0 caret-foreground outline-hidden prose-sm max-w-none",
          "data-testid": "message-input",
          spellcheck: "false",
        },
        // ArrowUp in an empty composer → edit your last message (Slack
        // parity). Handled here in ProseMirror's own DOM `keydown` hook —
        // NOT via `addKeyboardShortcuts` (the keymap plugin) and NOT via a
        // wrapper-level React `onKeyDown`.
        //
        // Why this layer specifically: immediately after a send the editor
        // still holds DOM focus and the doc was just cleared. In the app's
        // WebView, ProseMirror's keymap/vertical-arrow path does not reliably
        // route ArrowUp to our binding in that state — the keystroke is
        // effectively swallowed until the user clicks out and back (which is
        // exactly the reported bug). `handleKeyDown` is the first, lowest hook
        // ProseMirror exposes: it runs on the raw DOM keydown before any
        // command/caret logic, fires regardless of selection state, and works
        // the same across browser engines. Returning `true` consumes the key.
        handleKeyDown: (view, event) => {
          if (event.key !== "ArrowUp") return false;
          // Respect the same guards as before: no modifiers (let ⌥↑/⇧↑/etc.
          // through), autocomplete closed, a handler exists, and the composer
          // is empty (never steal the arrow from drafted text or an in-flight
          // edit, whose loaded body makes the doc non-empty).
          if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey)
            return false;
          if (isAutocompleteOpen?.current) return false;
          const handler = onEditLastOwnMessageRef.current;
          if (!handler) return false;
          // Emptiness is read straight off the live ProseMirror doc rather
          // than a captured `editor` ref — the `editor` instance isn't in
          // scope at config time (useEditor deps are `[]`), and the view's
          // state is always current. Empty = a single empty textblock with
          // no text content (mirrors Tiptap's `editor.isEmpty`).
          const { doc } = view.state;
          const isEmptyDoc =
            doc.childCount <= 1 && doc.textContent.length === 0;
          if (!isEmptyDoc) return false;
          // Consume only if a target was found and edit mode was entered;
          // otherwise let ArrowUp fall through to normal caret movement.
          return handler();
        },
      },
      onUpdate: ({ editor: ed }) => {
        const markdown = getMarkdownFromEditor(ed);
        // Use the same plain-text projection that `getPlainTextAndCursor`
        // uses, so autocomplete detection sees the *same* string the
        // cursor offset is mapped against. `state.doc.textContent` would
        // diverge by 1 per hard-break / block boundary.
        const text = buildPlainTextProjection(ed.state.doc).text;
        onUpdateRef.current?.({ markdown, text });
      },
    },
    [],
  );

  // Toggle editable without destroying the editor instance.
  //
  // When the composer is disabled mid-send (`isSending` flips the `disabled`
  // prop true), ProseMirror sets the underlying element `contenteditable=false`
  // and the browser BLURS it — focus jumps to `document.body`. When the send
  // completes and the editor becomes editable again, focus does NOT return on
  // its own. That left the just-emptied composer focus-less, so the very next
  // ArrowUp (edit-last-message) never reached the editor's keydown hook and
  // did nothing until the user clicked back in. We restore focus here, scoped
  // to *this* editor instance (we only refocus if this editor was the one that
  // lost focus to the disable), so it can't steal focus from another composer.
  const hadFocusBeforeDisableRef = React.useRef(false);
  React.useEffect(() => {
    if (!editor || editor.isEditable === editable) return;
    if (!editable) {
      // About to disable: remember whether we currently hold focus so we know
      // whether to restore it when re-enabled.
      hadFocusBeforeDisableRef.current = editor.isFocused;
      editor.setEditable(false);
    } else {
      editor.setEditable(true);
      // Re-enabled: if we owned focus before the disable blurred us, take it
      // back (preserving the current selection — `focus()` with no arg keeps
      // the existing selection rather than jumping to the end).
      if (hadFocusBeforeDisableRef.current) {
        hadFocusBeforeDisableRef.current = false;
        editor.commands.focus();
      }
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
      | { names: string[]; agentNames: string[]; channelNames: string[] }
      | undefined;
    if (storage) {
      storage.names = mentionNames ?? [];
      storage.agentNames = agentMentionNames ?? [];
      storage.channelNames = channelNames ?? [];
      // Force the plugin to re-decorate by dispatching a metadata transaction.
      const { tr } = editor.state;
      editor.view.dispatch(tr.setMeta(mentionHighlightKey, true));
    }
  }, [editor, mentionNames, agentMentionNames, channelNames]);

  // Custom-emoji set changes: re-resolve the `src` attr on any existing
  // node in the doc (e.g. an emoji's image was just published).
  React.useEffect(() => {
    if (!editor) return;
    customEmojiWiring.syncEmojiSrc(editor);
  }, [editor, customEmojiWiring.syncEmojiSrc]);

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

  const focusEnd = React.useCallback(() => {
    editor?.commands.focus("end");
  }, [editor]);

  /**
   * Ensure the editor has DOM focus without moving the ProseMirror
   * selection. If the editor already has focus this is a no-op.
   * Use this for re-render-triggered focus calls (e.g. reply-target
   * effect) where we don't want to yank the cursor to the end.
   */
  const focusPreserve = React.useCallback(() => {
    if (!editor) return;
    // `focus()` with no position argument preserves the current selection.
    editor.commands.focus();
  }, [editor]);

  // Backwards-compatible alias — existing call sites that want "end"
  // behaviour keep working. New call sites should use the explicit names.
  const focus = focusEnd;

  /**
   * Plain-text view of the document plus the cursor position in
   * plain-text offset space. Used by autocomplete detection (mentions,
   * channel links, emoji) which is shaped like a textarea.
   *
   * The plain-text projection treats both `hardBreak` and inter-block
   * boundaries as `\n` — matching `doc.textBetween(0, end, "\n", "\n")`.
   * See `plainTextProjection.ts`.
   */
  const getPlainTextAndCursor = React.useCallback((): {
    text: string;
    cursor: number;
  } => {
    if (!editor) return { text: "", cursor: 0 };
    const projection = buildPlainTextProjection(editor.state.doc);
    const anchor = editor.state.selection.anchor;
    return {
      text: projection.text,
      cursor: projection.mapPMToTextOffset(anchor),
    };
  }, [editor]);

  /**
   * Replace a plain-text range with literal text, in a single native
   * ProseMirror transaction.
   *
   * `fromOffset` and `toOffset` are in plain-text-offset space (the
   * same space as `getPlainTextAndCursor`). `text` is inserted verbatim
   * — including any trailing space — without a markdown re-parse.
   *
   * This replaces the old `setContentWithTrailingSpace` + full-doc
   * markdown round-trip used by autocomplete: by going through
   * `tr.insertText` we preserve active marks, hard breaks, list
   * structure, undo history continuity, and any whitespace.
   *
   * Returns the new cursor PM position, mapped through `tr.mapping` so
   * callers get a position that's valid after the transaction is
   * applied.
   */
  const replacePlainTextRange = React.useCallback(
    (
      fromOffset: number,
      toOffset: number,
      text: string,
      customEmojiShortcode?: string,
    ) => {
      if (!editor) return;
      const projection = buildPlainTextProjection(editor.state.doc);
      const fromPM = projection.mapTextOffsetToPM(fromOffset);
      const toPM = projection.mapTextOffsetToPM(toOffset);

      if (customEmojiShortcode) {
        // Replace the range with a CustomEmojiNode (the selectable/copyable
        // atom) followed by `text` (the trailing space). Equivalent to what
        // the input rule builds when the user types a known `:shortcode:`.
        const shortcode = customEmojiShortcode.toLowerCase();
        const emojiType = editor.schema.nodes[CUSTOM_EMOJI_NODE_NAME];
        if (emojiType) {
          const node = emojiType.create({
            shortcode,
            src: customEmojiWiring.resolveUrl(shortcode) ?? "",
          });
          let tr = editor.state.tr.replaceRangeWith(fromPM, toPM, node);
          // Insert the trailing space after the node, then place the cursor
          // after it.
          const afterNode = tr.mapping.map(toPM);
          if (text) tr = tr.insertText(text, afterNode);
          const cursorPM = afterNode + (text ? text.length : 0);
          tr = tr.setSelection(TextSelection.create(tr.doc, cursorPM));
          editor.view.dispatch(tr);
          editor.view.focus();
          return;
        }
        // No node type (shouldn't happen) → fall through to literal text.
      }

      const tr = editor.state.tr.insertText(text, fromPM, toPM);
      // Place cursor at the end of the inserted text. We map `toPM` (the
      // right end of the replaced range) through the transaction's
      // mapping — that's the post-transaction position right after the
      // inserted text, valid even if mark normalisation shifted things.
      // (Mapping `fromPM + text.length` directly would be a pre-image
      // position that may not exist in the original doc, which throws
      // "Position N out of range".)
      const cursorPM = tr.mapping.map(toPM);
      tr.setSelection(TextSelection.create(tr.doc, cursorPM));
      editor.view.dispatch(tr);
      editor.view.focus();
    },
    [editor, customEmojiWiring.resolveUrl],
  );

  return {
    editor,
    getMarkdown,
    isEmpty,
    clearContent,
    setContent,
    focus,
    focusEnd,
    focusPreserve,
    getPlainTextAndCursor,
    replacePlainTextRange,
  };
}

export type UseRichTextEditorResult = ReturnType<typeof useRichTextEditor>;

function getMarkdownFromEditor(editor: Editor): string {
  // biome-ignore lint/suspicious/noExplicitAny: tiptap-markdown storage is untyped
  const storage = (editor.storage as any).markdown as
    | { getMarkdown?: () => string }
    | undefined;
  if (storage?.getMarkdown) {
    let md = storage.getMarkdown();
    // tiptap-markdown serializes hard breaks as "\" + newline (CommonMark hard
    // line break syntax). Chat messages are plain text, not rendered markdown,
    // so strip the backslashes to keep clean newlines.
    md = md.replace(/\\\n/g, "\n");
    // prosemirror-markdown's esc() backslash-escapes markdown special characters
    // (` * \ ~ [ ] _) in text nodes to prevent them from being interpreted as
    // formatting. Since our messages ARE rendered as markdown, we want to
    // preserve the user's original characters so code fences, bold, etc. work.
    md = md.replace(/\\([`*\\~[\]_])/g, "$1");
    return md;
  }
  // Fallback: plain text
  return editor.state.doc.textContent;
}
