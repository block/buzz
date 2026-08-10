/**
 * Separator handling shared by identifier matchers (channel names, user
 * display names, mention labels). Identifiers use separators as cosmetic
 * word breaks — `a-channel-name` and `achannelname` are the same word to a
 * person — so matchers strip them on both query and candidate. Message
 * bodies are prose and must stay literal; never use this on content.
 */

/** Separators that delimit words in an identifier, including unicode dashes. */
export const WORD_SEPARATORS = /[\s\-_./\u2010-\u2015\u2212]+/;

const WORD_SEPARATORS_GLOBAL = new RegExp(WORD_SEPARATORS.source, "g");

/** Strip separators so `a-channel-name` and `achannelname` compare equal. */
export function collapseSeparators(value: string): string {
  return value.replace(WORD_SEPARATORS_GLOBAL, "");
}

/**
 * Whether every char of `query` appears in `text` in order (not necessarily
 * contiguously). e.g. `acn` is a subsequence of `a-channel-name`.
 */
export function isSubsequence(query: string, text: string): boolean {
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
