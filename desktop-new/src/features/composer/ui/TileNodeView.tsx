import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";

import { InlineTile } from "@/shared/ui/InlineTile";

import { tileAddressFromAttrs } from "../tileNode";

/**
 * Mounts the shared InlineTile inside the editor document.
 *
 * This is the seam that lets one component serve both the composer and the
 * conversation. It is deliberately thin: it reads the address off the node and
 * renders, holding no state of its own, so there is nothing here to drift from
 * the read-only rendering path.
 *
 * The tile is not interactive inside the composer. While writing, a click's
 * meaning is "put my caret here", not "open this person's profile" — and a
 * focusable control inside a contenteditable region takes the caret out of the
 * editor. Detail-on-click belongs to the conversation, where a click has no
 * competing meaning.
 */
export function TileNodeView({ node }: ReactNodeViewProps) {
  const address = tileAddressFromAttrs(node.attrs);

  return (
    // Note: TipTap's `NodeViewWrapper` spreads its own props onto the rendered
    // element, so `as` also lands as a literal DOM attribute. Cosmetic and
    // upstream; not worth patching the dependency over.
    <NodeViewWrapper as="span" className="tile-node-view">
      {address ? <InlineTile address={address} interactive={false} /> : null}
    </NodeViewWrapper>
  );
}
