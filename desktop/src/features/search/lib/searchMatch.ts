export type SearchMatchPart = {
  isMatch: boolean;
  key: string;
  text: string;
};

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Terms used by desktop prefix search. Completed whitespace-delimited terms
 * match whole words; the trailing term also matches word prefixes.
 */
export function getSearchHighlightTerms(query: string): string[] {
  const terms = query
    .trim()
    .split(/\s+/)
    .map((term) => term.replace(/^[^\p{L}\p{N}_]+|[^\p{L}\p{N}_+]+$/gu, ""))
    .filter(Boolean);

  return [...new Set(terms.map((term) => term.toLocaleLowerCase()))].sort(
    (left, right) => right.length - left.length,
  );
}

/** Split text around every case-insensitive token/prefix match of the query. */
export function splitSearchMatches(
  text: string,
  query: string,
): SearchMatchPart[] {
  const terms = getSearchHighlightTerms(query);
  if (terms.length === 0) {
    return [{ isMatch: false, key: "0", text }];
  }

  const pattern = new RegExp(`(${terms.map(escapeRegExp).join("|")})`, "giu");
  const termSet = new Set(terms);
  let offset = 0;
  return text
    .split(pattern)
    .filter(Boolean)
    .map((part) => {
      const key = `${offset}-${part.length}`;
      offset += part.length;
      return {
        isMatch: termSet.has(part.toLocaleLowerCase()),
        key,
        text: part,
      };
    });
}

/**
 * Build a compact result excerpt that keeps the first matching search term
 * visible. Context is biased before the match so the excerpt still reads like
 * a sentence while avoiding a match that is clipped offscreen.
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

  const normalizedText = text.toLocaleLowerCase();
  const matchIndex = getSearchHighlightTerms(query).reduce((earliest, term) => {
    const index = normalizedText.indexOf(term);
    return index >= 0 && (earliest < 0 || index < earliest) ? index : earliest;
  }, -1);
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
