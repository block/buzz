/**
 * Escape special regex characters in a string.
 */
export function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const NEVER_MATCH = /(?!)/gi;

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
  const boundary = "(?=[\\s,;.!?:)\\]}]|$)";
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
