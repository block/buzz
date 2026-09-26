import { mentionLabelPattern } from "./mentionBoundaries";
/**
 * Escape special regex characters in a string.
 */
export function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const NEVER_MATCH = /(?!)/gi;

/**
 * Build a regex that matches a given prefix followed by known multi-word names
 * (longest-first to avoid partial matches). An optional generic token pattern
 * stays available alongside known names, so channel chips do not depend on
 * membership or a loaded directory. Generic matching uses mobile's channel
 * boundaries to exclude URL fragments and trailing punctuation.
 *
 * Without a generic pattern, only known names match. In particular, arbitrary
 * @words are never highlighted as mentions without matching p-tagged names.
 */
export function buildPrefixPattern(
  prefix: string,
  knownNames: string[],
  options?: { genericTokenPattern?: string },
): RegExp {
  const sorted = [...new Set(knownNames)]
    .filter((name) => name.trim().length > 0)
    .sort((a, b) => b.length - a.length);

  const escapedPrefix = escapeRegExp(prefix);

  if (sorted.length === 0 && !options?.genericTokenPattern) {
    return NEVER_MATCH;
  }

  const nameAlternatives = sorted
    .map((name) =>
      prefix === "@" ? mentionLabelPattern(name) : escapeRegExp(name),
    )
    .join("|");
  const boundary = "(?=[\\s,;.!?:)\\]}]|$)";
  if (options?.genericTokenPattern) {
    const alternatives = [nameAlternatives, options.genericTokenPattern]
      .filter(Boolean)
      .join("|");
    return new RegExp(
      `(?<![\\w./:-])${escapedPrefix}(?:${alternatives})${boundary}`,
      "gi",
    );
  }
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
