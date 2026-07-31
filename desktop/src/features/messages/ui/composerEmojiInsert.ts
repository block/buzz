/**
 * Decides how a picked emoji enters the composer document.
 *
 * Split out of `MessageComposer.tsx` to keep that module under the desktop
 * file-size ceiling (see `desktop/scripts/check-file-sizes.mjs`).
 *
 * A `:shortcode:` for a known custom emoji becomes a selectable atom node (the
 * same node the input rule and autocomplete produce), so it can be selected,
 * copied, and deleted as one unit. Everything else — native unicode, or a
 * shortcode with no matching palette entry — inserts as plain content.
 */
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

export type EmojiInsertion =
  | { kind: "custom-node"; shortcode: string; src: string }
  | { kind: "text"; text: string };

export function resolveEmojiInsertion(
  emoji: string,
  customEmoji: ReadonlyArray<CustomEmoji>,
): EmojiInsertion {
  const shortcode = /^:([^:\s]+):$/.exec(emoji)?.[1]?.toLowerCase();
  if (!shortcode) return { kind: "text", text: emoji };

  const entry = customEmoji.find(
    (candidate) => candidate.shortcode.toLowerCase() === shortcode,
  );
  if (!entry) return { kind: "text", text: emoji };

  return { kind: "custom-node", shortcode, src: entry.url ?? "" };
}
