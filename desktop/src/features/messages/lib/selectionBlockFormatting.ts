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

function mapRangeThroughLatestStep(
  transaction: Transaction,
  from: number,
  to: number,
): { from: number; to: number } {
  const stepMap = transaction.steps.at(-1)?.getMap();
  return stepMap
    ? {
        from: stepMap.map(from, 1),
        to: stepMap.map(to, -1),
      }
    : { from, to };
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

  const isBackward = transaction.selection.anchor > transaction.selection.head;
  let { from, to } = transaction.selection;

  const nodeAfterSelection = transaction.doc.resolve(to).nodeAfter;
  if (nodeAfterSelection?.type.name === "hardBreak") {
    transaction.delete(to, to + nodeAfterSelection.nodeSize);
    ({ from, to } = mapRangeThroughLatestStep(transaction, from, to));
  }

  const nodeBeforeSelection = transaction.doc.resolve(from).nodeBefore;
  if (nodeBeforeSelection?.type.name === "hardBreak") {
    transaction.delete(from - nodeBeforeSelection.nodeSize, from);
    ({ from, to } = mapRangeThroughLatestStep(transaction, from, to));
  }

  if (canSplitInsideTextblock(transaction, to)) {
    transaction.split(to);
    ({ from, to } = mapRangeThroughLatestStep(transaction, from, to));
  }

  if (canSplitInsideTextblock(transaction, from)) {
    transaction.split(from);
    ({ from, to } = mapRangeThroughLatestStep(transaction, from, to));
  }

  transaction.setSelection(
    TextSelection.create(
      transaction.doc,
      isBackward ? to : from,
      isBackward ? from : to,
    ),
  );
  return true;
}
