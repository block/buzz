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
  // A caret belongs to the character *before* it, so resolve the link from
  // `nodeBefore` first. This disambiguates the seam between two adjacent
  // links: at that boundary both `marks()` and `nodeAfter` point at the
  // right-hand link, but the caret should anchor on the left one. When the
  // position is inside a text node (not on a boundary) `nodeBefore`/
  // `nodeAfter` are null, so we fall back to `marks()` for the mid-node case
  // and to `nodeAfter` for the caret at a link's very left edge.
  const markBefore = $pos.nodeBefore
    ? linkType.isInSet($pos.nodeBefore.marks)
    : null;
  const markAfter = $pos.nodeAfter
    ? linkType.isInSet($pos.nodeAfter.marks)
    : null;
  const onBoundary = $pos.nodeBefore != null || $pos.nodeAfter != null;
  const mark = onBoundary
    ? markBefore || markAfter
    : linkType.isInSet($pos.marks());
  if (!mark) return null;

  const rawHref = mark.attrs.href;
  if (typeof rawHref !== "string") return null;
  const href = rawHref;
  const parent = $pos.parent;
  const parentStart = $pos.start();

  type ChildSpan = { from: number; to: number; hasLink: boolean };
  const spans: ChildSpan[] = [];
  // Adjacent text nodes share a boundary (span N's `to` equals span N+1's
  // `from`), so a `pos` landing exactly on a seam falls inside both. Picking
  // the last match there would anchor on the wrong node when two links with
  // different hrefs abut. We resolve the ambiguity in two passes: first try
  // the span that both contains `pos` and carries our target link; only if
  // none does (e.g. caret resting just past a link) fall back to a plain
  // containment test with an exclusive right edge so the seam belongs to the
  // node on its left.
  let anchorIndex = -1;
  let linkAnchorIndex = -1;
  parent.forEach((child, childOffset) => {
    const childFrom = parentStart + childOffset;
    const childTo = childFrom + child.nodeSize;
    const childLink = linkType.isInSet(child.marks);
    const hasLink = childLink != null && childLink.attrs.href === href;
    const index = spans.length;
    if (childFrom <= pos && pos < childTo) anchorIndex = index;
    if (hasLink && childFrom <= pos && pos <= childTo) linkAnchorIndex = index;
    spans.push({ from: childFrom, to: childTo, hasLink });
  });
  if (linkAnchorIndex !== -1) anchorIndex = linkAnchorIndex;

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
