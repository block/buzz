import type { EditorState } from "@tiptap/pm/state";

export type LinkSelectionInfo = {
  href: string;
  text: string;
  from: number;
  to: number;
};

/**
 * Resolve the link mark covering position `pos`, expanded to the full
 * contiguous range of that same link. Returns the href, the covered text,
 * and the `from`/`to` document positions, or `null` when `pos` is not
 * inside a link.
 *
 * ProseMirror stores links as marks on text nodes, so one visual link can
 * span several adjacent text nodes when its text carries mixed formatting
 * (bold/italic/code). We extend outward from the child under `pos` across
 * every contiguous sibling carrying the same link href, so Edit/Remove
 * operate on the whole link rather than a single fragment.
 */
export function resolveLinkAt(
  state: EditorState,
  pos: number,
): LinkSelectionInfo | null {
  const linkType = state.schema.marks.link;
  if (!linkType) return null;

  const $pos = state.doc.resolve(pos);
  // The mark at the caret sits on the character *before* the position, with
  // the character after as a fallback (caret at a link's left edge).
  const mark =
    linkType.isInSet($pos.marks()) ||
    (pos < state.doc.content.size
      ? linkType.isInSet(state.doc.resolve(pos + 1).marks())
      : null);
  if (!mark) return null;

  const href = mark.attrs.href as string;
  const parent = $pos.parent;
  const parentStart = $pos.start();

  type ChildSpan = { from: number; to: number; hasLink: boolean };
  const spans: ChildSpan[] = [];
  let anchorIndex = -1;
  parent.forEach((child, childOffset) => {
    const childFrom = parentStart + childOffset;
    const childTo = childFrom + child.nodeSize;
    const childLink = linkType.isInSet(child.marks);
    const hasLink = childLink != null && childLink.attrs.href === href;
    if (childFrom <= pos && pos <= childTo) anchorIndex = spans.length;
    spans.push({ from: childFrom, to: childTo, hasLink });
  });

  if (anchorIndex === -1) return { href, text: "", from: pos, to: pos };

  let from = spans[anchorIndex].from;
  let to = spans[anchorIndex].to;
  for (let i = anchorIndex - 1; i >= 0 && spans[i].hasLink; i--) {
    from = spans[i].from;
  }
  for (let i = anchorIndex + 1; i < spans.length && spans[i].hasLink; i++) {
    to = spans[i].to;
  }

  const text = state.doc.textBetween(from, to, "\n", "\n");
  return { href, text, from, to };
}
