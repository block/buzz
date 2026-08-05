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

import { vaultEditorExtensions } from "@/features/documents/lib/editor/vaultEditorExtensions";
import { toDiskMarkdown } from "@/features/documents/lib/markdownEscapes";

export type UseVaultEditorOptions = {
  /** Called only for genuine user edits, with the disk-ready markdown. */
  onChange: (markdown: string) => void;
  /** Ctrl/Cmd+S. */
  onSave: () => void;
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

export function useVaultEditor({ onChange, onSave }: UseVaultEditorOptions) {
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

  /** Loads a document without marking it dirty. */
  const loadDocument = React.useCallback(
    (markdown: string) => {
      if (!editor) return;
      isLoadingRef.current = true;
      try {
        editor.commands.setContent(markdown, { emitUpdate: false });
      } finally {
        isLoadingRef.current = false;
      }
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

  return { editor, getMarkdown, loadDocument };
}
