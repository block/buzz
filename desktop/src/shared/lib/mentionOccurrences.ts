import { getMentionOffsets } from "./mentionBoundaries";

/** A literal label and its winning ranges, with ties retained for the caller. */
export function mentionOccurrences<T extends { displayName: string }>(
  text: string,
  candidates: readonly T[],
): Array<{ start: number; end: number; candidates: T[] }> {
  const byOffset = new Map<number, { end: number; candidates: T[] }>();
  for (const candidate of candidates) {
    const label = candidate.displayName.trim();
    if (!label) continue;
    for (const start of getMentionOffsets(text, label)) {
      const end = start + 1 + label.length;
      const previous = byOffset.get(start);
      if (!previous || end > previous.end) {
        byOffset.set(start, { end, candidates: [candidate] });
      } else if (end === previous.end) {
        previous.candidates.push(candidate);
      }
    }
  }
  const occurrences: Array<{ start: number; end: number; candidates: T[] }> =
    [];
  for (const [start, match] of [...byOffset].sort(([a], [b]) => a - b)) {
    // A label can contain an @; do not reinterpret its interior as a recipient.
    if (start < (occurrences.at(-1)?.end ?? 0)) continue;
    occurrences.push({ start, ...match });
  }
  return occurrences;
}
