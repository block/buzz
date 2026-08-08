import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

import { findAgentMentionDecorationEndingAt } from "./mentionHighlightExtension";

/**
 * Bypass native contenteditable mutation for the first ordinary character
 * typed after the separator following a highlighted agent mention.
 *
 * Packaged WebKit can move that character across the decorated mention
 * boundary and replace the separator. Dispatching the verified insertion as
 * a ProseMirror transaction keeps the document and DOM mutation in agreement.
 * Every other beforeinput shape falls through to ProseMirror's normal path.
 */
export function handleMentionBoundaryBeforeInput(
  view: EditorView,
  event: InputEvent,
): boolean {
  const insertedText = event.data;
  if (
    event.inputType !== "insertText" ||
    event.isComposing ||
    view.composing ||
    !insertedText ||
    Array.from(insertedText).length !== 1 ||
    /[\r\n]/.test(insertedText)
  ) {
    return false;
  }

  const { selection } = view.state;
  if (!selection.empty || !selection.$from.parent.inlineContent) return false;

  const { $from, from } = selection;
  if ($from.parentOffset < 2) return false;

  const textBeforeCaret = $from.parent.textBetween(
    0,
    $from.parentOffset,
    "\n",
    "\n",
  );
  if (!textBeforeCaret.endsWith(" ")) return false;

  const separatorPosition = from - 1;
  if (!findAgentMentionDecorationEndingAt(view.state, separatorPosition)) {
    return false;
  }

  event.preventDefault();
  const transaction = view.state.tr.insertText(insertedText, from, from);
  transaction.setSelection(
    TextSelection.create(transaction.doc, from + insertedText.length),
  );
  view.dispatch(transaction.scrollIntoView());
  return true;
}
