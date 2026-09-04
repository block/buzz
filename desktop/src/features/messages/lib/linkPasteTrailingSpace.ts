import { Extension } from "@tiptap/core";
import { Plugin, TextSelection } from "@tiptap/pm/state";

const PASTED_LINK_AT_END_RE =
  /(?:^|\s)((?:https?:\/\/|www\.)[^\s]+|(?:github\.com|linear\.app|drive\.google\.com|docs\.google\.com)\/[^\s]+)$/i;

function shouldAppendSpaceAfterPaste(text: string): boolean {
  const trimmedEnd = text.trimEnd();
  if (!trimmedEnd || trimmedEnd.length !== text.length) return false;
  return PASTED_LINK_AT_END_RE.test(trimmedEnd);
}

/**
 * Appends a trailing space after a paste that ends in a bare link, so the
 * caret lands outside the autolinked mark and the next typed character is
 * not swallowed into the link.
 */
export const LinkPasteTrailingSpace = Extension.create({
  name: "linkPasteTrailingSpace",

  addProseMirrorPlugins() {
    return [
      new Plugin({
        props: {
          handlePaste(view, event) {
            const pastedText = event.clipboardData?.getData("text/plain") ?? "";
            if (!shouldAppendSpaceAfterPaste(pastedText)) return false;

            window.setTimeout(() => {
              if (!view.dom.isConnected) return;
              const { state } = view;
              if (!state.selection.empty) return;

              const from = state.selection.from;
              if (from < state.doc.content.size) {
                const nextText = state.doc.textBetween(
                  from,
                  Math.min(state.doc.content.size, from + 1),
                  "\n",
                  "\n",
                );
                if (/^\s$/.test(nextText)) return;
              }

              let transaction = state.tr.insertText(" ", from, from);
              const linkMark = state.schema.marks.link;
              if (linkMark) {
                transaction = transaction.removeMark(from, from + 1, linkMark);
              }
              transaction = transaction.setSelection(
                TextSelection.create(transaction.doc, from + 1),
              );
              transaction.setStoredMarks([]);
              view.dispatch(transaction.scrollIntoView());
              view.focus();
            }, 0);

            return false;
          },
        },
      }),
    ];
  },
});
