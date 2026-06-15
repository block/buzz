import * as React from "react";

import { observeElementBlockSize } from "@/shared/layout/observeElementBlockSize";

/**
 * Observes the height of the composer overlay and sets the scroll
 * container's `paddingBottom` to match, so content is never hidden
 * behind the absolutely-positioned composer.
 *
 * If the user is pinned to the bottom (per the anchored-scroll hook's
 * truth, NOT a local threshold) when padding increases, auto-scrolls
 * to keep them at the bottom (no visible gap).
 *
 * @param atBottomRef Optional ref written by `useAnchoredScroll`; when
 *   provided, the at-bottom decision uses the hook's `AT_BOTTOM_THRESHOLD_PX`
 *   (currently 24) as the single source of truth. When absent, the
 *   composer falls back to its own 32px near-bottom check.
 */
export function useComposerHeightPadding(
  scrollContainerRef: React.RefObject<HTMLElement | null>,
  composerRef: React.RefObject<HTMLElement | null>,
  resetKey?: unknown,
  atBottomRef?: React.RefObject<boolean>,
) {
  React.useEffect(() => {
    void resetKey;
    const scrollEl = scrollContainerRef.current;
    const composerEl = composerRef.current;

    if (!scrollEl || !composerEl) {
      return;
    }

    const isAtBottom = (): boolean => {
      if (atBottomRef) return atBottomRef.current === true;
      // Fallback for callers that haven't wired the ref yet.
      const threshold = 32;
      return (
        scrollEl.scrollHeight - scrollEl.scrollTop - scrollEl.clientHeight <
        threshold
      );
    };

    let lastPadding: number | null = null;

    const applyPadding = (height: number) => {
      const padding = Math.ceil(height);
      if (lastPadding !== null && Math.abs(padding - lastPadding) <= 1) {
        return;
      }

      const previousPadding = lastPadding;
      const wasAtBottom = isAtBottom();

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
  }, [scrollContainerRef, composerRef, resetKey, atBottomRef]);
}
