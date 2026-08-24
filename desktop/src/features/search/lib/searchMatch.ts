export type SearchMatchPart = {
  isMatch: boolean;
  key: string;
  text: string;
};

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Split text around every case-insensitive literal match of the query. */
export function splitSearchMatches(
  text: string,
  query: string,
): SearchMatchPart[] {
  const trimmedQuery = query.trim();
  if (!trimmedQuery) {
    return [{ isMatch: false, key: "0", text }];
  }

  const pattern = new RegExp(`(${escapeRegExp(trimmedQuery)})`, "gi");
  let offset = 0;
  return text
    .split(pattern)
    .filter(Boolean)
    .map((part) => {
      const key = `${offset}-${part.length}`;
      offset += part.length;
      return {
        isMatch: part.toLowerCase() === trimmedQuery.toLowerCase(),
        key,
        text: part,
      };
    });
}

/**
 * Build a compact result excerpt that keeps the first literal match visible.
 * Context is biased slightly before the match so the result still reads like
 * a sentence while avoiding a snippet whose matching word is offscreen.
 */
export function buildSearchResultPreview(
  content: string,
  query: string,
  maxLength = 96,
): string {
  const text = content.trim();
  if (!text) {
    return "No message body.";
  }
  if (text.length <= maxLength) {
    return text;
  }

  const trimmedQuery = query.trim();
  const matchIndex = trimmedQuery
    ? text.toLowerCase().indexOf(trimmedQuery.toLowerCase())
    : -1;
  if (matchIndex < 0) {
    return `${text.slice(0, Math.max(0, maxLength - 3)).trimEnd()}...`;
  }

  const contextBefore = Math.min(32, Math.floor(maxLength / 3));
  let start = Math.max(0, matchIndex - contextBefore);
  const end = Math.min(text.length, start + maxLength);

  if (end === text.length) {
    start = Math.max(0, end - maxLength);
  }

  const prefix = start > 0 ? "..." : "";
  const suffix = end < text.length ? "..." : "";
  const available = Math.max(0, maxLength - prefix.length - suffix.length);
  const excerpt = text.slice(start, start + available).trim();

  return `${prefix}${excerpt}${suffix}`;
}
