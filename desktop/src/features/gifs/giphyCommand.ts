const GIPHY_COMMAND = /^\/giphy(?:[ \t]+([^\n]*))?$/i;

/**
 * Parse a `/giphy <query>` slash command from composer markdown.
 *
 * Returns the search query (empty for a bare `/giphy`, which browses
 * trending), or `null` when the text is not the command and should send as an
 * ordinary message. The editor's markdown serializer backslash-escapes
 * punctuation, so escapes are undone before the query reaches the search.
 */
export function parseGiphyCommand(text: string): string | null {
  const match = GIPHY_COMMAND.exec(text.trim());
  if (!match) return null;
  return (match[1] ?? "").replace(/\\(.)/g, "$1").trim();
}
