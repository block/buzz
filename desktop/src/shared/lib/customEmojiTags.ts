/**
 * Build NIP-30 `["emoji", shortcode, url]` tags for the custom shortcodes that
 * actually appear in an outgoing message body. Pure + testable; the resolved
 * emoji list comes from the community palette (see `customEmoji.ts`).
 *
 * Mirrors how @mentions become `p` tags: the tag set is derived from the final
 * content at send time so the event is self-contained — any NIP-30 client can
 * render the emoji (or fall back to the literal `:shortcode:` text).
 */

import type { CustomEmoji } from "./remarkCustomEmoji";

const SHORTCODE_SCAN = /:([a-z0-9_-]+):/gi;

/**
 * Return one `["emoji", shortcode, url]` tag per *distinct* known custom emoji
 * referenced in `content`. Shortcodes are matched case-insensitively against
 * the (lowercase) emoji set; the canonical lowercase shortcode is emitted.
 * Unknown `:foo:` sequences are ignored. Order follows first appearance; each
 * shortcode is emitted at most once.
 */
export function buildCustomEmojiTags(
  content: string,
  customEmoji: ReadonlyArray<CustomEmoji>,
): string[][] {
  if (customEmoji.length === 0) return [];
  const urlByShortcode = new Map(customEmoji.map((e) => [e.shortcode, e.url]));

  const emitted = new Set<string>();
  const tags: string[][] = [];

  SHORTCODE_SCAN.lastIndex = 0;
  let match: RegExpExecArray | null = null;
  while (true) {
    match = SHORTCODE_SCAN.exec(content);
    if (!match) break;
    const shortcode = match[1].toLowerCase();
    if (emitted.has(shortcode)) continue;
    const url = urlByShortcode.get(shortcode);
    if (!url) continue;
    emitted.add(shortcode);
    tags.push(["emoji", shortcode, url]);
  }

  return tags;
}
