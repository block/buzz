const MESSAGE_LINK_SNIPPET_MAX_LENGTH = 160;

/** Build a compact, non-recursive plain-text preview for a linked message. */
export function summarizeMessageLinkContent(content: string): string {
  const normalized = Array.from(content, (character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)
      ? " "
      : character;
  })
    .join("")
    .replace(/\|\|[^|]*(?:\|(?!\|)[^|]*)*\|\|/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/<?(?:https?|buzz):\/\/\S+>?/g, " ")
    .replace(/[`*_~>#|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!normalized) return "No message text";

  const characters = Array.from(normalized);
  if (characters.length <= MESSAGE_LINK_SNIPPET_MAX_LENGTH) return normalized;
  const clipped = characters
    .slice(0, MESSAGE_LINK_SNIPPET_MAX_LENGTH - 1)
    .join("");
  const lastSpace = clipped.lastIndexOf(" ");
  const snippet = lastSpace > 96 ? clipped.slice(0, lastSpace) : clipped;
  return `${snippet.trimEnd()}…`;
}
