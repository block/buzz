export type SlashQuery = {
  query: string;
  /** Plain-text offset of the `/`. */
  startIndex: number;
  /** Plain-text offset just past the first token. */
  endIndex: number;
};

/**
 * The autocomplete opens while the caret is in the first token, only
 * whitespace precedes it, and the token is `/` plus name characters.
 */
export function detectSlashQuery(
  text: string,
  cursor: number,
): SlashQuery | null {
  const startIndex = text.length - text.trimStart().length;
  if (text[startIndex] !== "/") return null;
  const rest = text.slice(startIndex);
  const tokenLength = rest.search(/\s/);
  const endIndex =
    startIndex + (tokenLength === -1 ? rest.length : tokenLength);
  const token = text.slice(startIndex, endIndex);
  if (!/^\/[\w-]*$/.test(token)) return null;
  if (cursor <= startIndex || cursor > endIndex) return null;
  return { query: token.slice(1), startIndex, endIndex };
}
