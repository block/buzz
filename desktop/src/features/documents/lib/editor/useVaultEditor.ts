/**
 * The live-preview markdown editor for a vault note.
 *
 * Modelled on `features/messages/lib/useRichTextEditor.ts`, but with one rule
 * that composer does not need: **a note the user has not edited must never be
 * written**. `onUpdate` therefore bails unless the transaction actually changed
 * the document, and every programmatic content swap passes `emitUpdate: false`.
 *
 * Onyx wires its listener to an event that also fires on load, which is exactly
 * how "open a file, touch nothing, and it silently rewrites on disk" happens.
 */
import * as React from "react";
import { useEditor } from "@tiptap/react";

import {
  cacheDocument,
  getCachedDocument,
} from "@/features/documents/lib/editor/documentJsonCache";
import { vaultEditorExtensions } from "@/features/documents/lib/editor/vaultEditorExtensions";
import {
  wikilinkKey,
  type WikilinkClickHandler,
  type WikilinkStorage,
} from "@/features/documents/lib/editor/wikilinkExtension";
import {
  obsidianSyntaxKey,
  type ObsidianSyntaxStorage,
} from "@/features/documents/lib/editor/obsidianSyntaxExtension";
import { toDiskMarkdown } from "@/features/documents/lib/markdownEscapes";
import type { NoteIndex } from "@/features/documents/lib/noteIndex";
import type { OutlineHeading } from "@/features/documents/lib/obsidianSyntax";

export type UseVaultEditorOptions = {
  /** Path of the note being edited, for same-folder wikilink resolution. */
  currentPath: string;
  /** Vault-wide index; `null` while the corpus is still loading. */
  noteIndex: NoteIndex | null;
  /** Called only for genuine user edits, with the disk-ready markdown. */
  onChange: (markdown: string) => void;
  /** Ctrl/Cmd+S. */
  onSave: () => void;
  onWikilinkClick: WikilinkClickHandler;
  /** Receives the heading list whenever the document changes. */
  onHeadingsChange?: (headings: OutlineHeading[]) => void;
  onTagClick?: (tag: string) => void;
};

function readMarkdown(editor: {
  storage: unknown;
  state: { doc: { textContent: string } };
}): string {
  const storage = editor.storage as {
    markdown?: { getMarkdown?: () => string };
  };
  const raw = storage.markdown?.getMarkdown?.();
  if (typeof raw !== "string") {
    // Falling back to plain text would silently flatten the document; refusing
    // is safer than writing a degraded version over the user's note.
    throw new Error("tiptap-markdown storage is unavailable");
  }
  return toDiskMarkdown(raw);
}

export function useVaultEditor({
  currentPath,
  noteIndex,
  onChange,
  onHeadingsChange,
  onSave,
  onTagClick,
  onWikilinkClick,
}: UseVaultEditorOptions) {
  const onChangeRef = React.useRef(onChange);
  onChangeRef.current = onChange;
  const onSaveRef = React.useRef(onSave);
  onSaveRef.current = onSave;

  /**
   * Suppresses `onChange` while we are loading a document into the editor.
   * `emitUpdate: false` covers most of it, but input rules and paste handling
   * can still dispatch during a swap.
   */
  const isLoadingRef = React.useRef(false);

  const editor = useEditor({
    extensions: vaultEditorExtensions(),
    content: "",
    editorProps: {
      attributes: {
        class: "documents-editor focus:outline-none",
        // Notes are prose; code spans opt out individually.
        spellcheck: "true",
      },
      handleKeyDown: (_view, event) => {
        const isSaveChord =
          (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s";
        if (!isSaveChord) return false;
        event.preventDefault();
        onSaveRef.current();
        return true;
      },
    },
    onUpdate: ({ editor: instance, transaction }) => {
      // The load path must not mark the note dirty.
      if (isLoadingRef.current) return;
      if (!transaction.docChanged) return;
      try {
        onChangeRef.current(readMarkdown(instance));
      } catch {
        // A serializer failure must not take the editor down mid-keystroke.
        // The buffer stays as the user typed it; the next save attempt surfaces
        // the error where it can be shown.
      }
    },
  });

  // Keep wikilink decorations in sync with the index. Mutating
  // `editor.storage.<name>` directly is required: the extension instance's
  // `.storage` getter returns a fresh spread-copy per access, so writes through
  // it are silently lost (see the note in useRichTextEditor.ts).
  React.useEffect(() => {
    if (!editor) return;
    const storage = (editor.storage as unknown as Record<string, unknown>)
      .documentsWikilink as WikilinkStorage | undefined;
    if (!storage) return;

    storage.currentPath = currentPath;
    storage.noteIndex = noteIndex;
    storage.onWikilinkClick = onWikilinkClick;
    // Force a re-decoration; the document itself has not changed.
    editor.view.dispatch(editor.state.tr.setMeta(wikilinkKey, true));
  }, [currentPath, editor, noteIndex, onWikilinkClick]);

  React.useEffect(() => {
    if (!editor) return;
    const storage = (editor.storage as unknown as Record<string, unknown>)
      .documentsObsidianSyntax as ObsidianSyntaxStorage | undefined;
    if (!storage) return;

    storage.onHeadingsChange = onHeadingsChange ?? null;
    storage.onTagClick = onTagClick ?? null;
    // Publish the current headings immediately; the plugin only emits on
    // change, so a freshly loaded document would otherwise show no outline.
    onHeadingsChange?.(storage.headings);
    editor.view.dispatch(editor.state.tr.setMeta(obsidianSyntaxKey, true));
  }, [editor, onHeadingsChange, onTagClick]);

  /** Loads a document without marking it dirty. */
  const loadDocument = React.useCallback(
    (markdown: string) => {
      if (!editor) return;
      isLoadingRef.current = true;
      try {
        // Re-parsing markdown dominates the cost of opening or switching to a
        // note; the identical parsed document is 30-47x cheaper to install.
        const cached = getCachedDocument(currentPath, markdown);
        if (cached !== null) {
          editor.commands.setContent(cached, { emitUpdate: false });
          return;
        }
        editor.commands.setContent(markdown, { emitUpdate: false });
        cacheDocument(currentPath, markdown, editor.getJSON());
      } finally {
        isLoadingRef.current = false;
      }
    },
    [currentPath, editor],
  );

  /**
   * Moves the caret to a document position and scrolls it into view. Used by
   * the outline panel, which knows heading positions but not the editor.
   */
  const scrollToPosition = React.useCallback(
    (position: number) => {
      if (!editor) return;
      editor.chain().focus().setTextSelection(position).scrollIntoView().run();
    },
    [editor],
  );

  /**
   * Vertical offsets of `positions` relative to the scroll container, for
   * scroll-spy. Returns an empty list when the view is not laid out yet.
   */
  const measureOffsets = React.useCallback(
    (positions: readonly number[]): number[] => {
      if (!editor?.view.dom.isConnected) return [];
      const container = editor.view.dom.parentElement;
      if (!container) return [];
      const containerTop = container.getBoundingClientRect().top;
      return positions.map((position) => {
        try {
          return (
            editor.view.coordsAtPos(position).top -
            containerTop +
            container.scrollTop
          );
        } catch {
          // A position can briefly be out of range mid-update.
          return Number.POSITIVE_INFINITY;
        }
      });
    },
    [editor],
  );

  const getMarkdown = React.useCallback((): string | null => {
    if (!editor) return null;
    try {
      return readMarkdown(editor);
    } catch {
      return null;
    }
  }, [editor]);

  return {
    editor,
    getMarkdown,
    loadDocument,
    measureOffsets,
    scrollToPosition,
  };
}
