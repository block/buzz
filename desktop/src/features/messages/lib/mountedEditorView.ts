import type { EditorView } from "@tiptap/pm/view";
import type { Editor } from "@tiptap/react";

/**
 * Resolve an editor's ProseMirror view, or `null` when it is not mounted.
 *
 * A tiptap v3 `Editor` outlives its view: before `EditorContent` mounts it and
 * after the subtree unmounts, `editor.view` is a proxy that *throws* for every
 * key it does not stub — including `dom`, `domAtPos`, and `coordsAtPos`. A
 * non-null `editor` therefore does not imply a usable view, so any code that
 * reaches past the editor into the view must go through this guard.
 */
export function getMountedView(editor: Editor): EditorView | null {
  if (editor.isDestroyed) return null;
  try {
    return editor.view.dom ? editor.view : null;
  } catch {
    // Throwing proxy — the view is detached right now.
    return null;
  }
}
