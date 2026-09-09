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
    const dom = editor.view.dom;
    const keydown = (event: KeyboardEvent) => {
      if (!["Enter", "Tab", " "].includes(event.key)) cancel();
    };
    editor.on("transaction", transaction);
    dom.addEventListener("keydown", keydown);
    dom.addEventListener("beforeinput", cancel);
    dom.addEventListener("pointerdown", cancel);
    return () => {
      cancel();
      editor.off("transaction", transaction);
      dom.removeEventListener("keydown", keydown);
      dom.removeEventListener("beforeinput", cancel);
      dom.removeEventListener("pointerdown", cancel);
    };
  }, [editor, cancel]);
}
