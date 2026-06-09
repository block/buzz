import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

/**
 * Build the emoji-mart `custom` prop from the workspace custom emoji palette.
 * Returns `undefined` when there are none (so the Picker shows only standard
 * categories). A selected custom emoji has no `native` field — only `id`/`src`
 * — so select handlers must insert `:id:` for these.
 */
export function buildCustomEmojiCategory(customEmoji: CustomEmoji[]) {
  if (customEmoji.length === 0) return undefined;
  return [
    {
      id: "sprout-custom",
      name: "Custom",
      emojis: customEmoji.map((e) => ({
        id: e.shortcode,
        name: `:${e.shortcode}:`,
        keywords: [e.shortcode],
        skins: [{ src: rewriteRelayUrl(e.url) }],
      })),
    },
  ];
}
