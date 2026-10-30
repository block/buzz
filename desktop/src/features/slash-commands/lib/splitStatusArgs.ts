export type StatusParts = {
  /** A native glyph, or `:shortcode:` for a custom emoji. Empty for none. */
  emoji: string;
  text: string;
};

export type StatusEmojiSources = {
  customEmoji: readonly { shortcode: string }[];
  /** Native glyph for a standard shortcode such as `tada`, or null. */
  standardEmoji: (shortcode: string) => string | null;
};

const SHORTCODE = /^:([^:\s]+):$/;

const SINGLE_EMOJI =
  /^(?:\p{Emoji_Presentation}|\p{Extended_Pictographic}️|[0-9#*]️?⃣)/u;

const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** Whether `token` is exactly one emoji grapheme, including ZWJ sequences. */
export function isSingleEmojiGrapheme(token: string): boolean {
  const iterator = graphemes.segment(token)[Symbol.iterator]();
  const first = iterator.next();
  if (first.done || !iterator.next().done) return false;
  return SINGLE_EMOJI.test(first.value.segment);
}

/** Whether `/status` arguments start with a `:shortcode:` that needs lookup. */
export function startsWithShortcode(args: string): boolean {
  return SHORTCODE.test(firstToken(args));
}

function firstToken(args: string): string {
  return args.trim().split(/\s/, 1)[0] ?? "";
}

/**
 * Splits `/status` arguments into an emoji and text. The first argument is the
 * emoji only when it is a single emoji grapheme or a `:shortcode:` that
 * resolves; a custom emoji wins over a standard one with the same shortcode.
 * Anything else makes the whole argument string the text.
 */
export function splitStatusArgs(
  args: string,
  sources: StatusEmojiSources,
): StatusParts {
  const trimmed = args.trim();
  const token = firstToken(trimmed);
  const emoji = resolveEmoji(token, sources);
  if (!emoji) return { emoji: "", text: trimmed };
  return { emoji, text: trimmed.slice(token.length).trim() };
}

function resolveEmoji(
  token: string,
  { customEmoji, standardEmoji }: StatusEmojiSources,
): string | null {
  const shortcode = SHORTCODE.exec(token)?.[1];
  if (shortcode === undefined) {
    return isSingleEmojiGrapheme(token) ? token : null;
  }
  const wanted = shortcode.toLowerCase();
  const custom = customEmoji.find(
    (entry) => entry.shortcode.toLowerCase() === wanted,
  );
  if (custom) return `:${custom.shortcode}:`;
  return standardEmoji(wanted);
}
