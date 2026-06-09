import type { Node as ProseMirrorNode } from "@tiptap/pm/model";

import { CUSTOM_EMOJI_NODE_NAME } from "./customEmojiNode";

/**
 * Plain-text projection of a ProseMirror document.
 *
 * Plain-text is what a textarea-shaped consumer (mention/channel/emoji
 * autocomplete) reads: hard breaks render as `\n`, and a single `\n`
 * separates content in different leaf blocks (paragraphs, list items,
 * etc.) — the same convention as `doc.textBetween(from, to, "\n", "\n")`.
 *
 * The same walk is used to map *both* directions:
 *  - PM position  → plain-text offset (`mapPMToTextOffset`)
 *  - text offset  → PM position       (`mapTextOffsetToPM`)
 *
 * Keeping a single source of truth means the two mappings can't drift,
 * which is the historic source of off-by-one bugs around `hardBreak` and
 * multi-block docs.
 *
 * Pure function — no editor / view / React dependency.
 */
export type PlainTextProjection = {
  /** The plain-text projection of the document. */
  text: string;
  /** Map a ProseMirror position to a plain-text offset. Clamps to [0, text.length]. */
  mapPMToTextOffset: (pm: number) => number;
  /** Map a plain-text offset to a ProseMirror position. Clamps to a valid in-doc position. */
  mapTextOffsetToPM: (offset: number) => number;
};

type Segment =
  // A text node: pm range and text range have equal length.
  | {
      kind: "text";
      pmFrom: number;
      pmTo: number;
      textFrom: number;
      textTo: number;
    }
  // A hardBreak: 1 PM position wide, contributes one `\n`.
  | {
      kind: "hardBreak";
      pmFrom: number;
      pmTo: number;
      textFrom: number;
      textTo: number;
    }
  // A boundary between two leaf-block siblings (paragraphs, list items,
  // headings, etc.) — zero PM positions wide. Both sides resolve to
  // `pmAt`, which is the boundary point between the two blocks (= start
  // of the next leaf-block's content, minus its opening token).
  | {
      kind: "blockBoundary";
      pmAt: number;
      textFrom: number;
      textTo: number;
    }
  // The interior of an empty leaf block — a zero-width "content slot"
  // at PM position `pmAt` and text offset `textAt`. Lets cursor / range
  // endpoints land inside empty paragraphs (paste, draft restore) and
  // round-trip correctly through the mapping.
  | {
      kind: "emptyBlockContent";
      pmAt: number;
      textAt: number;
    }
  // An inline atom leaf (e.g. a custom-emoji node) that is 1 PM position
  // wide but projects to its `:shortcode:` text. Unlike a `text` segment
  // the PM and text widths differ: PM spans [pmFrom, pmFrom+1] while text
  // spans the full shortcode. You can't place a cursor *inside* an atom,
  // so positions resolve to one edge or the other.
  | {
      kind: "atom";
      pmFrom: number;
      pmTo: number;
      textFrom: number;
      textTo: number;
    };

/**
 * Build a `PlainTextProjection` for the given doc.
 *
 * Walks the doc once and records each text node, hard break, and the
 * boundary between consecutive leaf-blocks. A "leaf block" is any block
 * node that does not itself contain blocks — `doc.textBetween` treats
 * exactly those boundaries as inserting the blockSeparator, and we do
 * the same so our text projection equals `doc.textBetween(0, end, "\n", "\n")`.
 */
export function buildPlainTextProjection(
  doc: ProseMirrorNode,
): PlainTextProjection {
  const segments: Segment[] = [];
  const textParts: string[] = [];
  let cursorText = 0;
  /**
   * True once we've entered at least one leaf-block. Subsequent leaf-blocks
   * emit a boundary `\n` before their content — matching `textBetween`.
   */
  let sawLeafBlock = false;

  doc.descendants((node, pos) => {
    // ── Leaf inline: text ──────────────────────────────────────────
    if (node.isText) {
      const t = node.text ?? "";
      segments.push({
        kind: "text",
        pmFrom: pos,
        pmTo: pos + t.length,
        textFrom: cursorText,
        textTo: cursorText + t.length,
      });
      textParts.push(t);
      cursorText += t.length;
      return false; // text nodes have no children
    }

    // ── Leaf inline: hard break ────────────────────────────────────
    if (node.type.name === "hardBreak") {
      segments.push({
        kind: "hardBreak",
        pmFrom: pos,
        pmTo: pos + 1,
        textFrom: cursorText,
        textTo: cursorText + 1,
      });
      textParts.push("\n");
      cursorText += 1;
      return false;
    }

    // ── Block ──────────────────────────────────────────────────────
    if (node.isBlock) {
      // Only "leaf blocks" (those with inline content — paragraphs,
      // headings, code blocks) produce text in `textBetween`'s sense.
      // Mixed-content blocks (lists, blockquotes) just contain other
      // blocks, so their *inner* leaf blocks record boundaries instead.
      const isLeafBlock = !!node.type.inlineContent;

      if (isLeafBlock) {
        if (sawLeafBlock) {
          // Boundary between the previous leaf-block and this one. The
          // PM position of the boundary is `pos` — the opening token of
          // the new leaf-block. textBetween emits the separator here.
          segments.push({
            kind: "blockBoundary",
            pmAt: pos,
            textFrom: cursorText,
            textTo: cursorText + 1,
          });
          textParts.push("\n");
          cursorText += 1;
        }
        sawLeafBlock = true;

        // If the leaf block is empty, no text/hardBreak segment will be
        // recorded inside it — but we still need a target for the
        // *interior* PM position so cursor / range endpoints sitting
        // inside the empty block round-trip correctly. Record a
        // zero-width content slot at `pos + 1` (= the position right
        // after this block's opening token).
        if (node.content.size === 0) {
          segments.push({
            kind: "emptyBlockContent",
            pmAt: pos + 1,
            textAt: cursorText,
          });
        }
      }
      return true; // descend into block children
    }

    // ── Leaf inline: custom-emoji atom ─────────────────────────────
    // 1 PM position wide, projects to its full `:shortcode:` text. Keeps
    // the two mappings consistent with what `renderText` emits, so cursor
    // math and autocomplete offsets see the shortcode at its natural width.
    if (node.type.name === CUSTOM_EMOJI_NODE_NAME) {
      const shortcode = String(node.attrs.shortcode ?? "");
      const projected = `:${shortcode}:`;
      segments.push({
        kind: "atom",
        pmFrom: pos,
        pmTo: pos + 1,
        textFrom: cursorText,
        textTo: cursorText + projected.length,
      });
      textParts.push(projected);
      cursorText += projected.length;
      return false;
    }

    // Other inline leaf nodes (none today) — skip silently.
    return true;
  });

  const text = textParts.join("");

  function mapPMToTextOffset(pm: number): number {
    if (pm <= 0) return 0;
    for (const seg of segments) {
      if (seg.kind === "text") {
        if (pm <= seg.pmTo) {
          if (pm <= seg.pmFrom) return seg.textFrom;
          return seg.textFrom + (pm - seg.pmFrom);
        }
      } else if (seg.kind === "hardBreak") {
        if (pm <= seg.pmFrom) return seg.textFrom;
        if (pm <= seg.pmTo) return seg.textTo;
      } else if (seg.kind === "atom") {
        // 1 PM wide; either before (pmFrom → textFrom) or after
        // (pmTo → textTo). No interior position exists.
        if (pm <= seg.pmFrom) return seg.textFrom;
        if (pm <= seg.pmTo) return seg.textTo;
      } else if (seg.kind === "blockBoundary") {
        // Zero PM-width.
        if (pm <= seg.pmAt) return seg.textFrom;
      } else {
        // emptyBlockContent — interior of an empty leaf block.
        if (pm <= seg.pmAt) return seg.textAt;
      }
    }
    return text.length;
  }

  function mapTextOffsetToPM(offset: number): number {
    if (offset <= 0) {
      // Position right inside the first content-carrying location.
      const first = segments.find(
        (s) =>
          s.kind === "text" ||
          s.kind === "hardBreak" ||
          s.kind === "atom" ||
          s.kind === "emptyBlockContent",
      );
      if (first) {
        return first.kind === "emptyBlockContent" ? first.pmAt : first.pmFrom;
      }
      return doc.content.size > 0 ? 1 : 0;
    }
    // Iterate segments; an offset that falls *exactly* at the right edge
    // of a separator segment (hardBreak / blockBoundary) is interpreted
    // as "start of the next content node" and so falls through to be
    // claimed by the next segment (a text segment, or an
    // emptyBlockContent slot if the next leaf-block is empty).
    for (const seg of segments) {
      if (seg.kind === "text") {
        if (offset <= seg.textTo) {
          return seg.pmFrom + (offset - seg.textFrom);
        }
      } else if (seg.kind === "hardBreak") {
        if (offset < seg.textTo) {
          // Anywhere before the right edge of the `\n` → before the break.
          return seg.pmFrom;
        }
        // offset === seg.textTo → fall through to the next segment.
      } else if (seg.kind === "atom") {
        // An offset inside the projected `:shortcode:` can't land within
        // the atom — snap to just before it. At the right edge fall
        // through so the position lands after the atom (pmTo, claimed by
        // the following segment or the end-of-doc tail).
        if (offset < seg.textTo) return seg.pmFrom;
        // offset === seg.textTo → fall through.
      } else if (seg.kind === "blockBoundary") {
        // Zero PM-width.
        // offset <  textTo → "end of previous block" → pmAt
        // offset === textTo → "start of next block" → fall through.
        if (offset < seg.textTo) return seg.pmAt;
      } else {
        // emptyBlockContent — interior of an empty leaf block.
        // offset === textAt → land inside the empty block at pmAt.
        if (offset <= seg.textAt) return seg.pmAt;
      }
    }
    // Beyond all content → end-of-doc text position.
    const last = segments[segments.length - 1];
    if (!last) return doc.content.size > 0 ? 1 : 0;
    if (last.kind === "text" || last.kind === "hardBreak") return last.pmTo;
    if (last.kind === "atom") return last.pmTo;
    if (last.kind === "emptyBlockContent") return last.pmAt;
    return last.pmAt;
  }

  return { text, mapPMToTextOffset, mapTextOffsetToPM };
}
