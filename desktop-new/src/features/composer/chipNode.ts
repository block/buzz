import { chipFaces } from "@/shared/chips/faceResolver";
import { mergeAttributes, Node } from "@tiptap/core";
import { ReactNodeViewRenderer } from "@tiptap/react";

import {
  formatChipAddress,
  isChipKind,
  type ChipAddress,
} from "@/shared/chips/address";

import { ChipNodeView } from "./ui/ChipNodeView";

export const CHIP_NODE_NAME = "chip";

/**
 * A chip in the composer document.
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
 *   chip and the very next keystroke REPLACES it, so the chip silently
 *   disappears mid-sentence. TipTap's own mention extension ships false for the
 *   same reason.
 * - `atom: true` is currently redundant — ProseMirror computes `isAtom` as
 *   `isLeaf || spec.atom` and a contentless node is already a leaf — and is
 *   kept as a statement of intent. It becomes load-bearing if a chip ever
 *   holds content.
 *
 * Whole-chip deletion needs no keyboard handler: Backspace from just after a
 * chip removes it entirely, natively. The spike proved this by removing a
 * hand-written handler and re-running.
 */
export const ChipNode = Node.create({
  name: CHIP_NODE_NAME,
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
    return [{ tag: "span[data-chip]" }];
  },

  renderHTML({ HTMLAttributes }) {
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-chip": "",
        "data-kind": undefined,
      }),
    ];
  },

  /**
   * The plain-text projection of a chip is its address, never its label.
   *
   * This is what a sent message carries and what an agent reads. A reader that
   * knows nothing about chips still receives a meaningful `buzz://` link rather
   * than broken markup, so the reference degrades rather than disappearing.
   */
  renderText({ node }) {
    const address = chipAddressFromAttrs(node.attrs);
    return address ? formatChipAddress(address) : "";
  },

  addStorage() {
    return {
      markdown: {
        serialize(
          state: { write: (text: string) => void },
          node: { attrs: Record<string, unknown> },
        ) {
          const address = chipAddressFromAttrs(node.attrs);
          if (address)
            state.write(
              `[${chipFaces.get(address).label.replace(/[[\]\\]/g, "")} ](${formatChipAddress(address)})`,
            );
        },
        parse: {
          updateDOM(element: HTMLElement) {
            for (const link of element.querySelectorAll('a[href^="buzz://"]')) {
              const match = link
                .getAttribute("href")
                ?.match(/^buzz:\/\/(agent|person)\/([a-f0-9]{64})$/);
              if (!match) continue;
              const chip = document.createElement("span");
              chip.setAttribute("data-chip", "");
              chip.setAttribute("data-kind", match[1]);
              chip.setAttribute("data-id", match[2]);
              link.replaceWith(chip);
            }
          },
        },
      },
    };
  },

  addNodeView() {
    return ReactNodeViewRenderer(ChipNodeView);
  },
});

/** Reads an address off node attributes, or null when they are not a valid one. */
export function chipAddressFromAttrs(
  attrs: Record<string, unknown>,
): ChipAddress | null {
  const kind = String(attrs.kind ?? "");
  const id = String(attrs.id ?? "");
  if (!isChipKind(kind) || id.length === 0) return null;
  return { kind, id };
}
