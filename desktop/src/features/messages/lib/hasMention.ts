/**
 * Escape special regex characters in a string.
 */
function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Check whether `text` contains an @mention of `name`.
 *
 * Matches `@Name` preceded by start-of-string, whitespace, an opening
 * parenthesis (for team expansions), markdown
 * bold/italic markers (`*`, `**`, `***`, `_`, `__`, `___`), or spoiler
 * delimiters (`||`). This handles the case where a mention is pasted from the
 * chat area and TipTap's Bold extension wraps it in bold marks (font-weight >=
 * 500 -> bold), plus messages whose visible mention text is spoilered.
 *
 * Exported separately so it can be unit-tested without importing React.
 */
export function getMentionOffset(text: string, name: string): number | null {
  const escaped = escapeRegExp(name);
  const pattern = new RegExp(
    `(^|\\s|\\(|[*_]{1,3}|\\|\\|)(@${escaped})(?=\\|\\||[\\s,;.!?:)\\]}*_]|$)`,
    "i",
  );
  const match = pattern.exec(text);
  return match ? match.index + match[1].length : null;
}

export function hasMention(text: string, name: string): boolean {
  return getMentionOffset(text, name) !== null;
}
