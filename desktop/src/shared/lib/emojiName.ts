import data from "@emoji-mart/data/sets/15/native.json" with { type: "json" };

type EmojiMartData = {
  emojis?: Record<
    string,
    {
      skins?: Array<{ native?: string }>;
    }
  >;
};

let shortcodeByNativeEmoji: Map<string, string> | null = null;

function buildShortcodeMap(): Map<string, string> {
  const map = new Map<string, string>();
  const emojis = (data as EmojiMartData).emojis ?? {};
  for (const [id, emoji] of Object.entries(emojis)) {
    const shortcode = `:${id}:`;
    for (const skin of emoji.skins ?? []) {
      if (skin.native && !map.has(skin.native)) {
        map.set(skin.native, shortcode);
      }
    }
  }
  return map;
}

export function emojiDisplayName(emoji: string): string {
  const trimmed = emoji.trim();
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) {
    return trimmed;
  }
  shortcodeByNativeEmoji ??= buildShortcodeMap();
  return shortcodeByNativeEmoji.get(trimmed) ?? trimmed;
}
