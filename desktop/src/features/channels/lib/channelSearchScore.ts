/**
 * Lightweight fuzzy matching for the channel browser search box.
 *
 * Mirrors the philosophy of `mentionRanking.ts`: cheap, dependency-free,
 * separator-aware scoring — no Levenshtein / typo-tolerance (which reorders
 * results unpredictably and hides the channel a user can plainly see).
 *
 * The one thing plain substring search gets wrong is contiguity across
 * separators: typing `releasenotes` or `reln` should still find
 * `release-notes`. We fix that with two extra passes on top of substring:
 *
 *  1. word-boundary tokens (split on space/`-`/`_`), so multi-word names match
 *     when the separators are dropped or a later word is typed, and
 *  2. an in-order subsequence check, so `reln` matches `release-notes`.
 *
 * Lower score === better match. `null` means "no match".
 */

/** Separators that delimit words in a channel name/description. */
const WORD_SEPARATORS = /[\s\-_./]+/;

// Score bands. Kept as named steps so the intent (and ordering) is legible.
const SCORE_EXACT = 0;
const SCORE_PREFIX = 1;
const SCORE_WORD_EXACT = 2;
const SCORE_WORD_PREFIX = 3;
const SCORE_SUBSTRING = 4;
const SCORE_COLLAPSED_SEPARATORS = 5;
const SCORE_SUBSEQUENCE = 6;
const SCORE_DESCRIPTION = 7;

/** Strip separators so `release-notes` and `releasenotes` compare equal. */
function collapseSeparators(value: string): string {
  return value.replace(/[\s\-_./]+/g, "");
}

/**
 * Whether every char of `query` appears in `text` in order (not necessarily
 * contiguously). e.g. `reln` is a subsequence of `release-notes`.
 */
function isSubsequence(query: string, text: string): boolean {
  if (query.length === 0) return true;
  let queryIndex = 0;
  for (const char of text) {
    if (char === query[queryIndex]) {
      queryIndex += 1;
      if (queryIndex === query.length) return true;
    }
  }
  return false;
}

/**
 * Score how well `name` matches `lowerQuery`. Returns the best (lowest) band,
 * or `null` if the name doesn't match at all. `lowerQuery` must already be
 * lowercased and trimmed.
 */
export function scoreChannelName(
  name: string,
  lowerQuery: string,
): number | null {
  if (lowerQuery.length === 0) return SCORE_EXACT;

  const lower = name.toLowerCase();

  if (lower === lowerQuery) return SCORE_EXACT;
  if (lower.startsWith(lowerQuery)) return SCORE_PREFIX;

  const words = lower.split(WORD_SEPARATORS).filter(Boolean);
  if (words.some((word) => word === lowerQuery)) return SCORE_WORD_EXACT;
  if (words.some((word) => word.startsWith(lowerQuery))) {
    return SCORE_WORD_PREFIX;
  }

  if (lower.includes(lowerQuery)) return SCORE_SUBSTRING;

  // `releasenotes` → matches `release-notes` once separators are removed.
  const collapsedName = collapseSeparators(lower);
  const collapsedQuery = collapseSeparators(lowerQuery);
  if (collapsedQuery.length > 0 && collapsedName.includes(collapsedQuery)) {
    return SCORE_COLLAPSED_SEPARATORS;
  }

  // `reln` → matches `release-notes` as an in-order subsequence. Guard against
  // 1-char queries producing noise by requiring at least 2 chars here.
  if (collapsedQuery.length >= 2 && isSubsequence(collapsedQuery, lower)) {
    return SCORE_SUBSEQUENCE;
  }

  return null;
}

export type ChannelSearchable = {
  name: string;
  description: string;
};

/**
 * Score a channel against a query, considering both name and description.
 * Description matches are always ranked below any name match. Returns `null`
 * when neither field matches.
 */
export function scoreChannelMatch(
  channel: ChannelSearchable,
  lowerQuery: string,
): number | null {
  if (lowerQuery.length === 0) return SCORE_EXACT;

  const nameScore = scoreChannelName(channel.name, lowerQuery);
  if (nameScore !== null) return nameScore;

  // Description only does plain substring — it's supplementary context, so we
  // don't want fuzzy description hits outranking or crowding out name matches.
  if (channel.description.toLowerCase().includes(lowerQuery)) {
    return SCORE_DESCRIPTION;
  }

  return null;
}
