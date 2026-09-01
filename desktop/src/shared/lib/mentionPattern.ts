/**
 * Escape special regex characters in a string.
 */
export function escapeRegExp(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const NEVER_MATCH = /(?!)/gi;

/**
 * Capture index of the leading boundary character in a pattern built by
 * {@link buildPrefixPattern} — pass it to `createRemarkPrefixPlugin` as
 * `leadGroup` so the character is re-emitted as text rather than swallowed
 * into the mention.
 */
export const PREFIX_LEAD_GROUP = 1;

/**
 * A prefix only opens a mention or channel link at the start of the text or
 * after whitespace or an opening bracket — `bob@alice.dev` is an address, not
 * a mention of `@alice`. The bracket set matches `detectPrefixQuery`, which
 * decides what opens an autocomplete query, so a name picked from the
 * autocomplete always renders as the mention it was tagged as.
 *
 * The boundary is a capture group rather than a lookbehind on purpose: WebKit
 * before Safari 16.4 fails to parse lookbehind and blanks the whole app
 * (#5547).
 */
const LEADING_BOUNDARY = "(^|[\\s([{])";

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
      return new RegExp(`${LEADING_BOUNDARY}${escapedPrefix}\\S+`, "gi");
    }
    return NEVER_MATCH;
  }

  const nameAlternatives = sorted.map((name) => escapeRegExp(name)).join("|");
  // A possessive still mentions the person, so the apostrophe closes a
  // mention — both the straight one and the curly U+2019 that macOS
  // substitutes while you type. `hasMention` (which decides the p-tag) uses
  // the same set; the two must agree or a tagged mention renders unstyled.
  const boundary = "(?=[\\s,;.!?:)\\]}'\u2019]|$)";
  return new RegExp(
    `${LEADING_BOUNDARY}${escapedPrefix}(?:${nameAlternatives})${boundary}`,
    "gi",
  );
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
