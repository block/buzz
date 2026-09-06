import { mergeAttributes, Node } from "@tiptap/core";

/**
 * Minimal inline tile for the composition-input spike.
 *
 * Deliberately the smallest thing that reproduces the risk: an
 * `inline` + `atom` node whose DOM is a non-editable box. That box is what
 * Chrome and Safari mis-handle at caret boundaries, and what a composing IME
 * has to survive sitting next to. Nothing here is production shape — no
 * resolver, no shared component, no address model. The spike answers one
 * question: does staged character input beside such a node stay intact?
 */
export const TILE_NODE_NAME = "spikeTile";

export const SpikeTileNode = Node.create({
  name: TILE_NODE_NAME,
  group: "inline",
  inline: true,
  /**
   * Redundant for this node and kept as documentation of intent. ProseMirror
   * computes `isAtom` as `isLeaf || spec.atom`, and a node with no content
   * schema is already a leaf — so flipping this to `false` changes nothing,
   * verified by running the spike with it off. It becomes load-bearing the
   * moment a tile holds content.
   */
  atom: true,
  /**
   * Load-bearing, and the single most important line in the file. With
   * `selectable: true` an arrow key selects the tile and the very next
   * keystroke REPLACES it — the tile silently vanishes mid-sentence. Verified
   * by flipping it: three spike tests go red. TipTap's own mention extension
   * ships `selectable: false` for this reason.
   */
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
      label: {
        default: "",
        parseHTML: (element) =>
          (element as HTMLElement).getAttribute("data-label") ?? "",
        renderHTML: (attributes) => ({
          "data-label": String(attributes.label),
        }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-spike-tile]" }];
  },

  renderHTML({ node, HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-spike-tile": "",
        contenteditable: "false",
        class: "spike-tile",
      }),
      `@${String(node.attrs.label)}`,
    ];
  },

  /**
   * The address, not the label. A spike-scale stand-in for the contract that
   * a sent message refers to the identity rather than the letters of a name.
   */
  renderText({ node }) {
    return `buzz://${String(node.attrs.kind)}/${String(node.attrs.id)}`;
  },

  /**
   * Whole-tile deletion needs NO custom handler. With `atom: true` and
   * `selectable: false`, Backspace from just after a tile removes it entirely —
   * verified by deleting this comment's former handler and re-running the spike
   * five times. TipTap's own mention extension adds a Backspace handler only to
   * change the behaviour (leave the trigger character behind), not to make
   * deletion work at all.
   */
});
