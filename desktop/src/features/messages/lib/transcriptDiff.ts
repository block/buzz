/** Minimal edit turning the currently shown phrase text into the next decode:
 *  keep the longest common prefix, replace only the tail. Live re-decodes are
 *  usually append-only (or identical, between words) — diffing means a stable
 *  decode touches nothing, so the caret and DOM stay still instead of being
 *  rewritten every ~300 ms. */
export function transcriptDiff(
  shown: string,
  next: string,
): { keep: number; deleteLen: number; insert: string } {
  let keep = 0;
  while (
    keep < shown.length &&
    keep < next.length &&
    shown.charCodeAt(keep) === next.charCodeAt(keep)
  ) {
    keep += 1;
  }
  return { keep, deleteLen: shown.length - keep, insert: next.slice(keep) };
}
