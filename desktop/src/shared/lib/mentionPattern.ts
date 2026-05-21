/**
 * Escape special regex characters in a string.
 */
export function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Build a regex that matches a given prefix followed by known multi-word names
 * (longest-first to avoid partial matches). When known names are provided,
 * only those names are matched — no generic fallback. When no names are
 * available, falls back to prefix + \S+ for backwards compatibility (e.g.
 * old messages without proper p-tags, or while profiles are loading).
 */
export function buildPrefixPattern(
  prefix: string,
  knownNames: string[],
): RegExp {
  const sorted = [...new Set(knownNames)]
    .filter((name) => name.trim().length > 0)
    .sort((a, b) => b.length - a.length);

  const escapedPrefix = escapeRegExp(prefix);

  if (sorted.length === 0) {
    return new RegExp(`${escapedPrefix}\\S+`, "gi");
  }

  const nameAlternatives = sorted.map((name) => escapeRegExp(name)).join("|");
  const boundary = "(?=[\\s,;.!?:)\\]}]|$)";
  return new RegExp(`${escapedPrefix}(?:${nameAlternatives})${boundary}`, "gi");
}

/**
 * Build a regex that matches @mentions, trying known multi-word names first
 * (longest-first to avoid partial matches), then falling back to @\S+.
 */
export function buildMentionPattern(mentionNames: string[]): RegExp {
  return buildPrefixPattern("@", mentionNames);
}
