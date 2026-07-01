import data from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { init } from "emoji-mart";
import * as React from "react";

import { buildCustomEmojiCategory } from "@/features/custom-emoji/emojiMartCategory";
import { useCustomEmoji } from "@/features/custom-emoji/hooks";

// emoji-mart builds its searchable index synchronously inside `init`, which
// `<Picker>` calls on mount — so the first reaction popover open paid the full
// ~1.8k-emoji index build and froze the cursor. Warm `init({ data })` once at
// idle so the index is prebuilt; `init` is a no-op after the first call (its
// `Data` singleton guards the rebuild), so the Picker's mount-time `init` skips
// the heavy work. Search still reads the prebuilt index — no first-keystroke
// hitch. Module-level so it fires regardless of when a picker first mounts.
let warmStarted = false;
function warmEmojiIndex() {
  if (warmStarted) {
    return;
  }
  warmStarted = true;
  const warm = () => void init({ data });
  if (typeof window !== "undefined" && "requestIdleCallback" in window) {
    window.requestIdleCallback(warm, { timeout: 1_500 });
  } else {
    globalThis.setTimeout(warm, 250);
  }
}
warmEmojiIndex();

/**
 * Reach into the `em-emoji-picker` shadow root and disable spellcheck,
 * autocorrect, and autocapitalize on its search input.
 *
 * emoji-mart mounts the custom element and its shadow content asynchronously,
 * so the input is not present on first render. A MutationObserver on the
 * shadow root watches for the input to appear, applies the attributes once,
 * then disconnects. Returns a cleanup function for `useEffect`.
 */
function disableSearchInputCorrections(host: HTMLElement): () => void {
  const picker = host.querySelector("em-emoji-picker");
  const shadowRoot = picker?.shadowRoot ?? null;
  if (!shadowRoot) {
    return () => undefined;
  }
  // shadowRoot is ShadowRoot here; capture as a typed const so the nested
  // closure doesn't re-widen to ShadowRoot | null.
  const root: ShadowRoot = shadowRoot;

  function applyAttributes() {
    const input = root.querySelector<HTMLInputElement>('input[type="search"]');
    if (!input) return false;
    input.spellcheck = false;
    input.setAttribute("autocorrect", "off");
    input.setAttribute("autocapitalize", "off");
    return true;
  }

  // Fast path: input already present (e.g. picker re-mount from hot cache).
  if (applyAttributes()) {
    return () => undefined;
  }

  // Slow path: wait for the shadow subtree to include the input.
  const observer = new MutationObserver(() => {
    if (applyAttributes()) {
      observer.disconnect();
    }
  });
  observer.observe(root, { childList: true, subtree: true });
  return () => observer.disconnect();
}

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
  const hostRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (!hostRef.current) return;
    return disableSearchInputCorrections(hostRef.current);
  }, []);

  return (
    <div ref={hostRef}>
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
    </div>
  );
});
