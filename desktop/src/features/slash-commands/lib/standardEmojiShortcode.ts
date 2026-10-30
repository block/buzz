import data from "@emoji-mart/data";

type EmojiMartData = {
  emojis: Record<string, { skins: { native: string }[] }>;
  aliases: Record<string, string>;
};

const { emojis, aliases } = data as EmojiMartData;

/** Native glyph for a lowercase standard shortcode or alias, or null. */
export function standardEmojiForShortcode(shortcode: string): string | null {
  const id = Object.hasOwn(emojis, shortcode)
    ? shortcode
    : Object.hasOwn(aliases, shortcode)
      ? aliases[shortcode]
      : null;
  return (id && emojis[id]?.skins[0]?.native) || null;
}
