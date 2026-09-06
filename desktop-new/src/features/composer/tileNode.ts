import { mergeAttributes, Node } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";

import {
  formatTileAddress,
  isTileKind,
  type TileAddress,
} from "@/shared/tiles/address";

import { TileNodeView } from "./ui/TileNodeView";

export const TILE_NODE_NAME = "tile";

/**
 * A tile in the composer document.
 *
 * The document stores an address — a kind and an identity — and nothing else.
 * No display name, because a name in the document would make every profile
 * rename an edit to the person's unsent message, dirtying the draft and
 * polluting undo history. The face is resolved at render time instead.
 *
 * Two flags carry the interaction contract, both verified by the composition
 * spike (see the Desktop New plan):
 *
 * - `selectable: false` is load-bearing. With it true, an arrow key selects the
 *   tile and the very next keystroke REPLACES it, so the tile silently
 *   disappears mid-sentence. TipTap's own mention extension ships false for the
 *   same reason.
 * - `atom: true` is currently redundant — ProseMirror computes `isAtom` as
 *   `isLeaf || spec.atom` and a contentless node is already a leaf — and is
 *   kept as a statement of intent. It becomes load-bearing if a tile ever
 *   holds content.
 *
 * Whole-tile deletion needs no keyboard handler: Backspace from just after a
 * tile removes it entirely, natively. The spike proved this by removing a
 * hand-written handler and re-running.
 */
export const TileNode = Node.create({
  name: TILE_NODE_NAME,
  group: "inline",
  inline: true,
  atom: true,
  selectable: false,

  addAttributes() {
    return {
      kind: {
        default: "person",
        parseHTML: (element) =>
          (element as HTMLElement).getAttribute("data-kind") ?? "person",
        renderHTML: (attributes) => ({ "data-kind": String(attributes.kind) }),
      },
      id: {
        default: "",
        parseHTML: (element) =>
          (element as HTMLElement).getAttribute("data-id") ?? "",
        renderHTML: (attributes) => ({ "data-id": String(attributes.id) }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-tile]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-tile": "",
        "data-kind": undefined,
      }),
    ];
  },

  /**
   * The plain-text projection of a tile is its address, never its label.
   *
   * This is what a sent message carries and what an agent reads. A reader that
   * knows nothing about tiles still receives a meaningful `buzz://` link rather
   * than broken markup, so the reference degrades rather than disappearing.
   */
  renderText({ node }) {
    const address = tileAddressFromAttrs(node.attrs);
    return address ? formatTileAddress(address) : "";
  },

  addNodeView() {
    return ReactNodeViewRenderer(TileNodeView);
  },
});

/** Reads an address off node attributes, or null when they are not a valid one. */
export function tileAddressFromAttrs(
  attrs: Record<string, unknown>,
): TileAddress | null {
  const kind = String(attrs.kind ?? "");
  const id = String(attrs.id ?? "");
  if (!isTileKind(kind) || id.length === 0) return null;
  return { kind, id };
}
