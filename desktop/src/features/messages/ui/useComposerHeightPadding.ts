import * as React from "react";

/**
 * Observes the height of the composer overlay and sets the scroll
 * container's `paddingBottom` to match, so content is never hidden
 * behind the absolutely-positioned composer.
 *
 * If the user is already scrolled to the bottom when padding increases,
 * auto-scrolls to keep them at the bottom (no visible gap).
 */
export function useComposerHeightPadding(
  scrollContainerRef: React.RefObject<HTMLElement | null>,
  composerRef: React.RefObject<HTMLElement | null>,
  resetKey?: unknown,
) {
  React.useEffect(() => {
    void resetKey;
    const scrollEl = scrollContainerRef.current;
    const composerEl = composerRef.current;

    if (!scrollEl || !composerEl || typeof ResizeObserver === "undefined") {
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
      const padding = Math.ceil(height);
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

    const observer = new ResizeObserver(([entry]) => {
      const height =
        entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
      applyPadding(height);
    });

    observer.observe(composerEl);
    applyPadding(composerEl.getBoundingClientRect().height);

    return () => {
      observer.disconnect();
      // Reset to a sensible default when unmounting
      scrollEl.style.paddingBottom = "";
    };
  }, [scrollContainerRef, composerRef, resetKey]);
}
