import data from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import * as React from "react";

import { buildCustomEmojiCategory } from "@/features/custom-emoji/emojiMartCategory";
import { useCustomEmoji } from "@/features/custom-emoji/hooks";

/**
 * The one emoji picker for the whole app. Every place that lets a user choose
 * an emoji — composing a message, reacting to a regular or system message,
 * setting a status — renders this, so the config and custom-emoji wiring can't
 * drift across call sites (they used to, and that's why custom emoji were
 * missing from some pickers).
 *
 * It always wires the workspace custom-emoji palette in via `useCustomEmoji()`,
 * so custom emoji show up everywhere for free. Selection is normalized to a
 * single string: a standard emoji emits its `native` glyph; a custom emoji has
 * no `native`, so it emits its `:shortcode:` (the emoji-mart `id` is the
 * shortcode). Consumers store/send that string and let the existing renderers
 * (reactions' `emojiUrl`, the remark shortcode plugin) resolve it to an image.
 *
 * Only the raw picker lives here — not the Popover/trigger. Those differ per
 * site (ghost button vs status swatch, popover vs dialog content) and forcing
 * them into one wrapper would be less clear, not more. The thing that drifted
 * was the picker config + custom wiring + select handling; that's what this
 * centralizes.
 */
type EmojiPickerProps = {
  /** Autofocus the search field when the picker mounts (e.g. reaction popovers). */
  autoFocus?: boolean;
  /** Called with the chosen emoji as a string: `native` glyph or `:shortcode:`. */
  onSelect: (emoji: string) => void;
};

export const EmojiPicker = React.memo(function EmojiPicker({
  autoFocus = false,
  onSelect,
}: EmojiPickerProps) {
  const customEmoji = useCustomEmoji();
  const custom = React.useMemo(
    () => buildCustomEmojiCategory(customEmoji),
    [customEmoji],
  );

  return (
    <Picker
      autoFocus={autoFocus}
      custom={custom}
      data={data}
      maxFrequentRows={2}
      onEmojiSelect={(emoji: { native?: string; id?: string }) => {
        // Standard emoji carry a `native` glyph. Custom emoji don't — emit
        // their `:shortcode:` (emoji-mart `id` == shortcode) instead. Ignore a
        // malformed selection that has neither.
        const value = emoji.native ?? (emoji.id ? `:${emoji.id}:` : "");
        if (value) {
          onSelect(value);
        }
      }}
      perLine={8}
      previewPosition="bottom"
      set="native"
      skinTonePosition="search"
      theme="auto"
    />
  );
});
