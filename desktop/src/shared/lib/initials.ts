const graphemeSegmenter =
  typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;

/** The first user-perceived character of a word, or "" when it has none. */
function firstGrapheme(word: string): string {
  if (graphemeSegmenter) {
    for (const { segment } of graphemeSegmenter.segment(word)) {
      return segment;
    }
    return "";
  }
  // Older engines without Intl.Segmenter degrade to a code point, which is
  // still whole — never half a surrogate pair. Mirrors `MessageLinkPill`.
  return Array.from(word)[0] ?? "";
}

/**
 * Derive up to two uppercase initials from a display name.
 *
 * An initial is a grapheme cluster, not a code unit and not a code point.
 * Taking `word[0]` returned half a surrogate pair for a name outside the
 * Basic Multilingual Plane; taking one code point returned a bare consonant
 * for `कुमार` or `မောင်`, and dropped the accent from a decomposed `Élodie`.
 * Only a cluster keeps a letter together with what belongs to it.
 *
 * Word separation keeps combining marks and join controls, which are neither
 * `\p{L}` nor `\p{N}`: replacing them with a separator cut words apart from
 * the inside, splitting `अनिल` at its vowel sign and the joined cluster
 * `क्‍ष` at its ZWJ.
 */
export function getInitials(name: string): string {
  return name
    .replace(/[^\p{L}\p{M}\p{N}\p{Join_Control}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .map(firstGrapheme)
    .slice(0, 2)
    .join("")
    .toUpperCase();
}
