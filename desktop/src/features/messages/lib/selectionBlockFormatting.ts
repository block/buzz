import { TextSelection, type Transaction } from "@tiptap/pm/state";
import { canSplit } from "@tiptap/pm/transform";

function canSplitInsideTextblock(
  transaction: Transaction,
  position: number,
): boolean {
  const $position = transaction.doc.resolve(position);
  return (
    $position.parent.inlineContent &&
    $position.parentOffset > 0 &&
    $position.parentOffset < $position.parent.content.size &&
    canSplit(transaction.doc, position)
  );
}

/**
 * Isolate a non-empty text selection at exact block boundaries.
 *
 * ProseMirror's block commands operate on whole textblocks. The composer can
 * hold an entire draft in one paragraph, so toggling a list or code block for
 * a substring otherwise formats the whole draft. Splitting at the selection
 * end and start first gives the selected text its own block while preserving
 * the surrounding content as sibling paragraphs.
 *
 * This mutates the transaction supplied by a Tiptap command chain so the
 * isolation and the following block toggle remain one undoable edit.
 */
export function isolateSelectionForBlockFormatting(
  transaction: Transaction,
): boolean {
  if (
    !(transaction.selection instanceof TextSelection) ||
    transaction.selection.empty
  ) {
    return false;
  }

  let { from, to } = transaction.selection;

  if (canSplitInsideTextblock(transaction, to)) {
    transaction.split(to);
    const splitMap = transaction.steps.at(-1)?.getMap();
    if (splitMap) {
      from = splitMap.map(from, 1);
      to = splitMap.map(to, -1);
    }
  }

  if (canSplitInsideTextblock(transaction, from)) {
    transaction.split(from);
    const splitMap = transaction.steps.at(-1)?.getMap();
    if (splitMap) {
      from = splitMap.map(from, 1);
      to = splitMap.map(to, -1);
    }
  }

  transaction.setSelection(TextSelection.create(transaction.doc, from, to));
  return true;
}
