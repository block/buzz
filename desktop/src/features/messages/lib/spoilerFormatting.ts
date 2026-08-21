import type { Editor } from "@tiptap/react";
import type { MarkType, Node as ProseMirrorNode } from "@tiptap/pm/model";

import { SPOILER_MARK_NAME } from "./spoilerMark";

export type SpoilerRangeState =
  | "fully-spoiled"
  | "partially-spoiled"
  | "no-markable-content";

function canTextNodeHoldMark(
  node: ProseMirrorNode,
  parent: ProseMirrorNode | null,
  markType: MarkType,
): boolean {
  if (!node.isText || node.textContent.length === 0) return false;
  if (parent && !parent.type.allowsMarkType(markType)) return false;
  if (markType.isInSet(node.marks)) return true;

  return node.marks.every((mark) => !mark.type.excludes(markType));
}

export function getSpoilerRangeState(
  doc: ProseMirrorNode,
  spoilerMark: MarkType,
  from: number,
  to: number,
): SpoilerRangeState {
  let hasMarkableContent = false;
  let isFullySpoiled = true;

  doc.nodesBetween(from, to, (node, _pos, parent) => {
    if (!canTextNodeHoldMark(node, parent, spoilerMark)) return;

    hasMarkableContent = true;
    if (!spoilerMark.isInSet(node.marks)) {
      isFullySpoiled = false;
      return false;
    }
  });

  if (!hasMarkableContent) return "no-markable-content";
  return isFullySpoiled ? "fully-spoiled" : "partially-spoiled";
}

export function getEditorSpoilerRangeState(
  editor: Editor,
  from: number,
  to: number,
): SpoilerRangeState {
  const spoilerMark = editor.schema.marks[SPOILER_MARK_NAME];
  if (!spoilerMark) return "no-markable-content";

  return getSpoilerRangeState(editor.state.doc, spoilerMark, from, to);
}
