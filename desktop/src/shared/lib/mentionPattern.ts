/**
 * Escape special regex characters in a string.
 */
export function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const NEVER_MATCH = /(?!)/gi;

/**
 * CJK / Hangul / Kana code-point ranges treated as a mention terminator.
 *
 * Display names are effectively Latin/ASCII, so a script transition from the
 * name straight into a CJK character (e.g. `@Fizz이렇게` with no separating
 * space) is an unambiguous word boundary. Without these ranges the boundary
 * lookahead only accepts whitespace/punctuation, so a mention immediately
 * followed by Korean/Japanese/Chinese text fails to match — the p-tag is never
 * attached and the notification never fires. Covers Hangul Jamo, Kana, Hangul
 * Compatibility Jamo, CJK Unified Ideographs, and Hangul Syllables.
 */
export const CJK_BOUNDARY_RANGES =
  "\\u1100-\\u11FF\\u3040-\\u30FF\\u3130-\\u318F\\u4E00-\\u9FFF\\uAC00-\\uD7A3";

/**
 * Build a regex that matches a given prefix followed by known multi-word names
 * (longest-first to avoid partial matches). When known names are provided,
 * only those names are matched — no generic fallback.
 *
 * When no names are available:
 * - If `options.fallbackToGeneric` is true, falls back to `prefix + \S+` so
 *   that patterns like `#channel` still render while channel names are loading
 *   asynchronously (used by remarkChannelLinks).
 * - Otherwise returns a never-matching regex, preventing arbitrary `@word`
 *   patterns from being highlighted as valid mentions when no p-tags are
 *   present (used by remarkMentions / buildMentionPattern).
 */
export function buildPrefixPattern(
  prefix: string,
  knownNames: string[],
  options?: { fallbackToGeneric?: boolean },
): RegExp {
  const sorted = [...new Set(knownNames)]
    .filter((name) => name.trim().length > 0)
    .sort((a, b) => b.length - a.length);

  const escapedPrefix = escapeRegExp(prefix);

  if (sorted.length === 0) {
    if (options?.fallbackToGeneric) {
      return new RegExp(`${escapedPrefix}\\S+`, "gi");
    }
    return NEVER_MATCH;
  }

  const nameAlternatives = sorted.map((name) => escapeRegExp(name)).join("|");
  const boundary = `(?=[\\s,;.!?:)\\]}${CJK_BOUNDARY_RANGES}]|$)`;
  return new RegExp(`${escapedPrefix}(?:${nameAlternatives})${boundary}`, "gi");
}

/**
 * Build a regex that matches @mentions for known multi-word names
 * (longest-first to avoid partial matches). When no known names are provided,
 * returns a never-matching regex — @word patterns are not highlighted unless
 * they correspond to an actual p-tagged member.
 */
export function buildMentionPattern(mentionNames: string[]): RegExp {
  return buildPrefixPattern("@", mentionNames);
}
