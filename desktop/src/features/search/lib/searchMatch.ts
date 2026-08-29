export type SearchMatchPart = {
  isMatch: boolean;
  key: string;
  text: string;
};

type SearchHighlightTerm = {
  isPrefix: boolean;
  value: string;
};

type TextLexeme = {
  end: number;
  normalized: string;
  start: number;
};

// PostgreSQL's `simple` text-search configuration breaks ordinary punctuation
// into lexemes (for example, `foo-bar` contributes `foo` and `bar`). Keep the
// desktop highlighter on those lexical boundaries rather than treating raw
// whitespace tokens as unrestricted substrings.
const LEXEME_PATTERN = /[\p{L}\p{N}]+/gu;

function extractLexemes(value: string): string[] {
  return Array.from(value.matchAll(LEXEME_PATTERN), (match) =>
    match[0].toLowerCase(),
  );
}

function getSearchHighlightMatchers(query: string): SearchHighlightTerm[] {
  const rawTokens = query.trim().split(/\s+/).filter(Boolean);
  const matchers: SearchHighlightTerm[] = [];

  rawTokens.forEach((rawToken, tokenIndex) => {
    const isPrefix = tokenIndex === rawTokens.length - 1;
    for (const value of extractLexemes(rawToken)) {
      matchers.push({ isPrefix, value });
    }
  });

  // Deduplicate repeated constraints without collapsing exact and prefix modes:
  // `foo foo` asks Postgres for both an exact `foo` and a `foo:*` lexeme.
  const deduped = new Map<string, SearchHighlightTerm>();
  for (const matcher of matchers) {
    deduped.set(
      `${matcher.isPrefix ? "prefix" : "exact"}:${matcher.value}`,
      matcher,
    );
  }
  return [...deduped.values()].sort(
    (left, right) => right.value.length - left.value.length,
  );
}

/**
 * Lexemes used by desktop prefix search after punctuation normalization.
 * Completed whitespace-delimited tokens match exactly; only lexemes from the
 * trailing token match prefixes.
 */
export function getSearchHighlightTerms(query: string): string[] {
  return getSearchHighlightMatchers(query).map((matcher) => matcher.value);
}

function getTextLexemes(text: string): TextLexeme[] {
  return Array.from(text.matchAll(LEXEME_PATTERN), (match) => ({
    end: (match.index ?? 0) + match[0].length,
    normalized: match[0].toLowerCase(),
    start: match.index ?? 0,
  }));
}

function getOriginalPrefixLength(
  original: string,
  normalizedLength: number,
): number {
  let normalizedOffset = 0;
  let originalOffset = 0;

  for (const character of original) {
    normalizedOffset += character.toLowerCase().length;
    originalOffset += character.length;
    if (normalizedOffset >= normalizedLength) {
      return originalOffset;
    }
  }

  return original.length;
}

function getMatchSpans(
  text: string,
  query: string,
): Array<{ end: number; start: number }> {
  const matchers = getSearchHighlightMatchers(query);
  if (matchers.length === 0) {
    return [];
  }

  const spans: Array<{ end: number; start: number }> = [];
  for (const lexeme of getTextLexemes(text)) {
    const exactMatch = matchers.find(
      (matcher) => !matcher.isPrefix && matcher.value === lexeme.normalized,
    );
    if (exactMatch) {
      spans.push({ start: lexeme.start, end: lexeme.end });
      continue;
    }

    const prefixMatch = matchers.find(
      (matcher) =>
        matcher.isPrefix && lexeme.normalized.startsWith(matcher.value),
    );
    if (prefixMatch) {
      const originalLexeme = text.slice(lexeme.start, lexeme.end);
      spans.push({
        start: lexeme.start,
        end:
          lexeme.start +
          getOriginalPrefixLength(originalLexeme, prefixMatch.value.length),
      });
    }
  }

  return spans;
}

/** Split text around case-insensitive lexeme/prefix matches of the query. */
export function splitSearchMatches(
  text: string,
  query: string,
): SearchMatchPart[] {
  const spans = getMatchSpans(text, query);
  if (spans.length === 0) {
    return [{ isMatch: false, key: "0", text }];
  }

  const parts: SearchMatchPart[] = [];
  let offset = 0;
  for (const span of spans) {
    if (span.start > offset) {
      parts.push({
        isMatch: false,
        key: `${offset}-${span.start - offset}`,
        text: text.slice(offset, span.start),
      });
    }
    parts.push({
      isMatch: true,
      key: `${span.start}-${span.end - span.start}`,
      text: text.slice(span.start, span.end),
    });
    offset = span.end;
  }
  if (offset < text.length) {
    parts.push({
      isMatch: false,
      key: `${offset}-${text.length - offset}`,
      text: text.slice(offset),
    });
  }
  return parts;
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

  const matchIndex = getMatchSpans(text, query)[0]?.start ?? -1;
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
