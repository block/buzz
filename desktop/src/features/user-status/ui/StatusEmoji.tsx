import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { cn } from "@/shared/lib/cn";
import { emojiDisplayName } from "@/shared/lib/emojiName";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

/**
 * Render a user-status emoji from its stored string. A status emoji is a bare
 * string (unlike reactions, which carry a companion `emojiUrl`): a native glyph
 * like `💬`, or a custom-emoji `:shortcode:`. This resolves a known shortcode to
 * its workspace image and renders it as an `<img>`; anything else (native glyph,
 * or an unknown `:foo:`) renders as text.
 *
 * Every place that shows a status emoji renders this, so the shortcode→image
 * resolution can't drift across the (five) display sites — the same reason the
 * picker is unified. The relay URL is rewritten through the localhost media
 * proxy, matching reactions' `EmojiGlyph` (WKWebView bypasses WARP, so a direct
 * relay URL 403s and renders broken).
 */
type StatusEmojiProps = {
  /** The stored status emoji: a native glyph or a custom `:shortcode:`. */
  value: string | undefined;
  /** Sizes the resolved custom image to match the surrounding text. */
  className?: string;
};

const SHORTCODE_RE = /^:([^:\s]+):$/;

export function StatusEmoji({ value, className }: StatusEmojiProps) {
  const customEmoji = useCustomEmoji();

  if (!value) return null;

  const displayName = emojiDisplayName(value);
  const match = value.match(SHORTCODE_RE);
  if (match) {
    const shortcode = match[1].toLowerCase();
    const found = customEmoji.find(
      (e) => e.shortcode.toLowerCase() === shortcode,
    );
    if (found) {
      return (
        <img
          alt={value}
          title={displayName}
          src={rewriteRelayUrl(found.url)}
          className={cn("inline-block object-contain align-middle", className)}
          draggable={false}
        />
      );
    }
  }

  // Native glyph, or an unknown shortcode we can't resolve — render as text.
  // Thread the caller's className through so native statuses keep the spacing
  // (e.g. `mr-1`) every display site applies to the image branch above.
  return (
    <span
      className={cn(
        "inline-flex items-center justify-center leading-normal align-middle",
        className,
      )}
      title={displayName}
    >
      {value}
    </span>
  );
}
