import * as React from "react";

import { observeElementBlockSize } from "@/shared/layout/observeElementBlockSize";

/**
 * Observes the height of the composer overlay and sets the scroll
 * container's `paddingBottom` to match, plus optional extra breathing room, so
 * content is never hidden behind the absolutely-positioned composer.
 *
 * If the user is already scrolled to the bottom when padding increases,
 * auto-scrolls to keep them at the bottom (no visible gap).
 */
export function useComposerHeightPadding(
  scrollContainerRef: React.RefObject<HTMLElement | null>,
  composerRef: React.RefObject<HTMLElement | null>,
  resetKey?: unknown,
  extraPaddingPx = 0,
) {
  React.useEffect(() => {
    void resetKey;
    const scrollEl = scrollContainerRef.current;
    const composerEl = composerRef.current;

    if (!scrollEl || !composerEl) {
      return;
    }

    const isNearBottom = (): boolean => {
      const threshold = 32;
      return (
        scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight <
        threshold
      );
    };

    let lastPadding: number | null = null;

    const applyPadding = (height: number) => {
      const padding = Math.ceil(height + extraPaddingPx);
      if (lastPadding !== null && Math.abs(padding - lastPadding) <= 1) {
        return;
      }

      const previousPadding = lastPadding;
      const wasAtBottom = isNearBottom();

      scrollEl.style.paddingBottom = `${padding}px`;
      lastPadding = padding;

      if (
        wasAtBottom &&
        (previousPadding === null || padding > previousPadding)
      ) {
        scrollEl.scrollTop = scrollEl.scrollHeight;
      }
    };

    const disconnect = observeElementBlockSize(composerEl, applyPadding);

    return () => {
      disconnect();
      scrollEl.style.paddingBottom = "";
    };
  }, [scrollContainerRef, composerRef, resetKey, extraPaddingPx]);
}
