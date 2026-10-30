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

/** Lowercase `:name:` tokens in `content`, distinct, in first-appearance order. */
export function collectEmojiShortcodes(content: string): string[] {
  const names = new Set<string>();
  for (const match of content.matchAll(SHORTCODE_SCAN)) {
    names.add(match[1].toLowerCase());
  }
  return [...names];
}

/** The part of the catalog resolver the send path needs. */
export type EmojiNameResolver = {
  resolve(names: ReadonlyArray<string>): Promise<{
    found: ReadonlyMap<string, CustomEmoji>;
    failed: boolean;
  }>;
};

/**
 * Like `buildCustomEmojiTags`, but names missing from the legacy list resolve
 * through `resolver` in one batched lookup. A name that does not resolve gets no
 * tag and stays plain text. `lookupFailed` is true when the lookup itself
 * failed, so the caller can tell "no such emoji" from "could not check".
 */
export async function resolveCustomEmojiTags(
  content: string,
  customEmoji: ReadonlyArray<CustomEmoji>,
  resolver: EmojiNameResolver,
): Promise<{ tags: string[][]; lookupFailed: boolean }> {
  const names = collectEmojiShortcodes(content);
  if (names.length === 0) return { tags: [], lookupFailed: false };
  const known = new Map(customEmoji.map((e) => [e.shortcode, e]));
  const unknown = names.filter((name) => !known.has(name));
  const { found, failed } =
    unknown.length > 0
      ? await resolver.resolve(unknown)
      : { found: new Map<string, CustomEmoji>(), failed: false };
  const tags: string[][] = [];
  for (const name of names) {
    const emoji = known.get(name) ?? found.get(name);
    if (emoji) tags.push(["emoji", name, emoji.url]);
  }
  return { tags, lookupFailed: failed };
}
