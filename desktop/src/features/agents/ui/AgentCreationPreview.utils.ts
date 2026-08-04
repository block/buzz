import * as React from "react";

import { useEmojiMartStyles } from "@/features/profile/ui/ProfileAvatarEditor.utils";

export function useFocusedEmojiPicker(
  containerRef: React.RefObject<HTMLDivElement | null>,
  enabled: boolean,
) {
  useEmojiMartStyles(containerRef, enabled);

  // Emoji Mart mounts its search input inside a shadow root. Wait for it
  // before focusing so the surrounding Radix popover cannot win the race.
  React.useEffect(() => {
    if (!enabled) {
      return;
    }

    let animationFrame = 0;
    const focusSearchInput = () => {
      const searchInput =
        containerRef.current
          ?.querySelector("em-emoji-picker")
          ?.shadowRoot?.querySelector<HTMLInputElement>(
            'input[type="search"]',
          ) ?? null;
      if (!searchInput) {
        animationFrame = window.requestAnimationFrame(focusSearchInput);
        return;
      }
      searchInput.focus();
    };

    animationFrame = window.requestAnimationFrame(focusSearchInput);
    return () => window.cancelAnimationFrame(animationFrame);
  }, [containerRef, enabled]);
}

export function isAvatarFileDrag(event: React.DragEvent<HTMLElement>) {
  return Array.from(event.dataTransfer.types).includes("Files");
}

export const AVATAR_APPLY_MOTION_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

export type AvatarTab = "image" | "emoji";

export type EmojiMartEmoji = {
  native?: string;
};
