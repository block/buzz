/**
 * Shared helper for detecting prefix-based queries (e.g. @mention, #channel)
 * within a text input, used by both useMentions and useChannelLinks hooks.
 */

/**
 * Detect whether the user is typing a prefix-based query (e.g. @name or #channel)
 * at the current cursor position.
 *
 * @param prefix - The trigger character (e.g. "@" or "#")
 * @param value - The full text content
 * @param cursorPosition - The current cursor position in the text
 * @param knownNamesLower - Lower-cased known names for multi-word prefix matching
 * @returns The query string and start index, or null if no query is detected
 */
export function detectPrefixQuery(
  prefix: string,
  value: string,
  cursorPosition: number,
  knownNamesLower: string[],
): { query: string; startIndex: number } | null {
  const beforeCursor = value.slice(0, cursorPosition);

  // A prefix only triggers a query when it opens a "word" — i.e. it sits at the
  // start of the text or right after whitespace or an opening bracket. Opening
  // brackets are included so `(#channel`, `[@name`, `{#chan` all autocomplete
  // the same as ` #channel` (previously only whitespace/start counted, so a
  // prefix glued to a `(` never fired). Keep the two detection paths below in
  // sync with this single definition of "boundary before the prefix".
  const isBoundaryChar = (ch: string) => /[\s([{]/.test(ch);

  // Fast path: single-word query (no spaces after the prefix)
  const escapedPrefix = prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const simplePattern = new RegExp(`(?:^|[\\s([{])${escapedPrefix}([^\\s]*)$`);
  const simpleMatch = beforeCursor.match(simplePattern);
  if (simpleMatch) {
    const query = simpleMatch[1];
    const startIndex = beforeCursor.length - query.length - 1; // -1 for prefix
    return { query, startIndex };
  }

  // Multi-word path: scan backwards for the prefix and check if the text between
  // the prefix and the cursor is a prefix of any known multi-word name.
  const scanStart = Math.max(0, beforeCursor.length - 80);
  for (let i = beforeCursor.length - 1; i >= scanStart; i--) {
    const ch = beforeCursor[i];
    if (ch === prefix) {
      // Ensure prefix is at start or preceded by whitespace/opening bracket
      if (i > 0 && !isBoundaryChar(beforeCursor[i - 1])) {
        continue;
      }
      const candidate = beforeCursor.slice(i + 1);
      if (candidate.length === 0) {
        break;
      }
      const lowerCandidate = candidate.toLowerCase();
      // A trailing space after an exact known name means the mention is
      // complete — don't keep the query open just because a longer name
      // (e.g. a team) shares the prefix.
      if (
        lowerCandidate.endsWith(" ") &&
        knownNamesLower.includes(lowerCandidate.trimEnd())
      ) {
        break;
      }
      const isPrefix = knownNamesLower.some((name) =>
        name.startsWith(lowerCandidate),
      );
      if (isPrefix) {
        return { query: candidate, startIndex: i };
      }
      break;
    }
    // Stop scanning if we hit a newline
    if (ch === "\n") {
      break;
    }
  }

  return null;
}
