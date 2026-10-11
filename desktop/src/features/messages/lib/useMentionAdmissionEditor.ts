import * as React from "react";
import type { Editor } from "@tiptap/react";

/** Retire choices on editor departures, including programmatic restoration. */
export function useMentionAdmissionEditor(
  editor: Editor | null,
  container: React.RefObject<HTMLElement | null>,
  cancel: () => void,
  onSelectionChange: () => void = cancel,
) {
  React.useEffect(() => {
    if (!editor) return;
    const onSelection = ({
      transaction,
    }: {
      transaction: { docChanged: boolean };
    }) => {
      if (!transaction.docChanged) onSelectionChange();
    };
    const onTransaction = ({
      transaction,
    }: {
      transaction: { getMeta: (key: string) => unknown };
    }) => {
      if (transaction.getMeta("preventUpdate")) cancel();
    };
    const onBlur = ({ event }: { event: FocusEvent }) => {
      if (
        !(event.relatedTarget instanceof Node) ||
        !container.current?.contains(event.relatedTarget)
      )
        cancel();
    };
    editor.on("selectionUpdate", onSelection);
    editor.on("transaction", onTransaction);
    editor.on("blur", onBlur);
    return () => {
      editor.off("selectionUpdate", onSelection);
      editor.off("transaction", onTransaction);
      editor.off("blur", onBlur);
      cancel();
    };
  }, [editor, container, cancel, onSelectionChange]);
}
