import type { Node as ProseMirrorNode } from "@tiptap/pm/model";

import { getMentionOffsets } from "./hasMention";

/**
 * Where the `@Label` runs of a document range actually sit.
 *
 * A pasted identity is fenced by the text it owns, and what it owns is the
 * mention token — not the whole sentence the paste happened to carry. Holding
 * the token means an edit elsewhere in the paste costs nothing, while any edit
 * to the token itself revokes the identity riding on it.
 *
 * Spans are read out of the *document* rather than out of the clipboard's own
 * text, because the document is what the fence has to hold: the Markdown
 * branch inserts source that TipTap parses, so `**@John Smith**` on the
 * clipboard is `@John Smith` in the doc. `getMentionOffsets` is the matcher
 * every other mention layer uses, so a span exists exactly where a send would
 * find a mention.
 */

/** One `@Label` run, in document coordinates. */
export type MentionTokenSpan = {
  label: string;
  from: number;
  to: number;
};

/** Stands in for text a range carries that no single position holds. */
const NO_POSITION = -1;

type RangeText = {
  text: string;
  /** Document position of each character, or `NO_POSITION`. */
  positions: number[];
};

/**
 * The text of `[from, to)` beside the document position of each character.
 *
 * Mirrors `doc.textBetween(from, to, "\n", "\n")` so the matcher reads what
 * the rest of the mention machinery reads, and records where every character
 * came from so a match converts back into document coordinates. Block
 * boundaries and non-text leaves contribute a newline owned by no position: a
 * label cannot span one, and a run that did would own no contiguous range.
 */
function readRangeText(
  doc: ProseMirrorNode,
  from: number,
  to: number,
): RangeText {
  const chars: string[] = [];
  const positions: number[] = [];
  let firstBlock = true;
  doc.nodesBetween(from, to, (node, pos) => {
    if (node.isText) {
      const value = node.text ?? "";
      const start = Math.max(from, pos);
      const end = Math.min(to, pos + value.length);
      for (let at = start; at < end; at += 1) {
        chars.push(value[at - pos]);
        positions.push(at);
      }
      return false;
    }
    if (node.isTextblock) {
      if (firstBlock) firstBlock = false;
      else {
        chars.push("\n");
        positions.push(NO_POSITION);
      }
      return true;
    }
    if (node.isLeaf) {
      chars.push("\n");
      positions.push(NO_POSITION);
      return false;
    }
    return true;
  });
  return { text: chars.join(""), positions };
}

/**
 * Locate every `@label` token of `labels` inside `[range.from, range.to)`.
 *
 * Spans come back in document order, so a caller that has to bound how many it
 * keeps drops the ones furthest into the range rather than starving a label
 * that happens to sort late.
 */
export function findMentionTokenSpans(
  doc: ProseMirrorNode,
  range: { from: number; to: number },
  labels: readonly string[],
): MentionTokenSpan[] {
  const from = Math.max(0, Math.min(range.from, doc.content.size));
  const to = Math.max(from, Math.min(range.to, doc.content.size));
  if (to <= from) return [];

  const { text, positions } = readRangeText(doc, from, to);
  const spans: MentionTokenSpan[] = [];
  const seen = new Set<string>();
  for (const label of labels) {
    // `getMentionOffsets` returns the offset of the sigil, and the match is
    // case-insensitive rather than fuzzy, so the run is always `@` + label.
    const length = label.length + 1;
    for (const offset of getMentionOffsets(text, label)) {
      const end = offset + length;
      if (end > positions.length) continue;
      const start = positions[offset];
      const last = positions[end - 1];
      // A run broken by a block boundary or a non-text node owns no single
      // range; skip it rather than inventing one that spans the break.
      if (start === NO_POSITION || last === NO_POSITION) continue;
      if (last - start !== end - 1 - offset) continue;
      const key = `${start} ${last}`;
      if (seen.has(key)) continue;
      seen.add(key);
      spans.push({ label, from: start, to: last + 1 });
    }
  }
  return spans.sort((a, b) => a.from - b.from);
}
