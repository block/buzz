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
) {
  React.useEffect(() => {
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

    const observer = new ResizeObserver(([entry]) => {
      const height =
        entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
      // Add a small buffer (8px) so the last message isn't flush against the composer
      const padding = Math.ceil(height) + 8;
      const wasAtBottom = isNearBottom();

      scrollEl.style.paddingBottom = `${padding}px`;

      if (wasAtBottom) {
        scrollEl.scrollTop = scrollEl.scrollHeight;
      }
    });

    observer.observe(composerEl);

    return () => {
      observer.disconnect();
      // Reset to a sensible default when unmounting
      scrollEl.style.paddingBottom = "";
    };
  }, [scrollContainerRef, composerRef]);
}
