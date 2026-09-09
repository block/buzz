import * as React from "react";
import type { Editor } from "@tiptap/react";

/** Native edits and selection transactions abandon authority, even after undo/return. */
export function useMentionAdmissionEditor(
  editor: Editor | null,
  cancel: () => void,
) {
  React.useEffect(() => {
    if (!editor) return;
    const transaction = ({
      transaction,
    }: {
      transaction: { docChanged: boolean; selectionSet: boolean };
    }) => {
      if (transaction.docChanged || transaction.selectionSet) cancel();
    };
    let dom: HTMLElement | null = null;
    const keydown = (event: KeyboardEvent) => {
      if (!["Enter", "Tab", " "].includes(event.key)) cancel();
    };
    const detach = () => {
      cancel();
      editor.off("transaction", transaction);
      dom?.removeEventListener("keydown", keydown);
      dom?.removeEventListener("beforeinput", cancel);
      dom?.removeEventListener("pointerdown", cancel);
      dom = null;
    };
    const attach = () => {
      if (dom) detach();
      // TipTap's public isDestroyed is also true when there is no view.
      // isInitialized/create are delayed until after mount, so cannot gate this.
      if (editor.isDestroyed) return;
      dom = editor.view.dom;
      editor.on("transaction", transaction);
      dom.addEventListener("keydown", keydown);
      dom.addEventListener("beforeinput", cancel);
      dom.addEventListener("pointerdown", cancel);
    };
    editor.on("mount", attach);
    editor.on("unmount", detach);
    editor.on("destroy", detach);
    attach();
    return () => {
      editor.off("mount", attach);
      editor.off("unmount", detach);
      editor.off("destroy", detach);
      detach();
    };
  }, [editor, cancel]);
}
