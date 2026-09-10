import { isSingleNativeEmoji } from "@/shared/lib/emojiOnly";

const WRAPPED_SHORTCODE = /^:([a-z0-9_-]+):$/i;

export type ReactionGlyphPresentation =
  | { kind: "native"; text: string }
  | { kind: "text"; text: string };

/**
 * Chooses the no-image reaction fallback. A native emoji gets the compact glyph
 * treatment; every other relay-valid reaction value gets text layout instead.
 */
export function reactionGlyphPresentation(
  emoji: string,
): ReactionGlyphPresentation {
  if (isSingleNativeEmoji(emoji)) {
    return { kind: "native", text: emoji };
  }

  const shortcode = emoji.match(WRAPPED_SHORTCODE)?.[1];
  return { kind: "text", text: shortcode ?? emoji };
}
