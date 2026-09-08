import type { Editor } from "@tiptap/react";

/** Returns the editor DOM only after TipTap has mounted its view. */
export function getMountedEditorDom(editor: Editor): HTMLElement | null {
  try {
    return editor.view.dom;
  } catch {
    return null;
  }
}
