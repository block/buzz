import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

/**
 * Build the emoji-mart `custom` prop: the bounded legacy palette plus the
 * locally cached recent and frequent catalog emoji, which emoji-mart's own
 * Recent and Frequent rows cannot show because they are not in its static list.
 * Returns `undefined` when there are none (so the Picker shows only standard
 * categories). A selected custom emoji has no `native` field — only `id`/`src`
 * — so select handlers must insert `:id:` for these.
 */
export function buildCustomEmojiCategory(
  customEmoji: CustomEmoji[],
  usage: { recent: CustomEmoji[]; frequent: CustomEmoji[] } = {
    recent: [],
    frequent: [],
  },
) {
  const toEmojiMart = (e: CustomEmoji) => ({
    id: e.shortcode,
    name: `:${e.shortcode}:`,
    keywords: [e.shortcode],
    skins: [{ src: rewriteRelayUrl(e.url) }],
  });
  const categories = [
    { id: "buzz-recent", name: "Recent custom", emojis: usage.recent },
    { id: "buzz-frequent", name: "Frequent custom", emojis: usage.frequent },
    { id: "buzz-custom", name: "Custom", emojis: customEmoji },
  ]
    .filter((category) => category.emojis.length > 0)
    .map((category) => ({
      ...category,
      emojis: category.emojis.map(toEmojiMart),
    }));
  return categories.length > 0 ? categories : undefined;
}
