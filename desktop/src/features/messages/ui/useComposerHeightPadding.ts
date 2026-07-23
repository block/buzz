import * as React from "react";

import { observeElementBlockSize } from "@/shared/layout/observeElementBlockSize";

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
  mode: "padding" | "css-variable" = "padding",
) {
  React.useEffect(() => {
    void resetKey;
    const scrollEl = scrollContainerRef.current;
    const composerEl = composerRef.current;

    if (!scrollEl || !composerEl) {
      return;
    }

    const getScrollElement = (): HTMLElement =>
      mode === "css-variable"
        ? (scrollEl.querySelector<HTMLElement>(
            '[data-testid="message-timeline"]',
          ) ?? scrollEl)
        : scrollEl;

    let lastPadding: number | null = null;
    let followBottomFrame: number | null = null;

    const isNearBottom = (): boolean => {
      const target = getScrollElement();
      const threshold = 32;
      const trailingClearance =
        mode === "css-variable" ? (lastPadding ?? 0) : 0;
      return (
        target.scrollHeight -
          target.scrollTop -
          target.clientHeight -
          trailingClearance <
        threshold
      );
    };

    const followBottom = () => {
      const target = getScrollElement();
      target.scrollTop = target.scrollHeight;
    };

    const applyPadding = (height: number) => {
      const padding = Math.ceil(height);
      if (lastPadding !== null && Math.abs(padding - lastPadding) <= 1) {
        return;
      }

      const previousPadding = lastPadding;
      const wasAtBottom = isNearBottom();

      if (mode === "css-variable") {
        scrollEl.style.setProperty("--composer-overlay-height", `${padding}px`);
      } else {
        scrollEl.style.paddingBottom = `${padding}px`;
      }
      lastPadding = padding;

      if (
        wasAtBottom &&
        (previousPadding === null || padding > previousPadding)
      ) {
        followBottom();
        if (followBottomFrame !== null) {
          cancelAnimationFrame(followBottomFrame);
        }
        followBottomFrame = requestAnimationFrame(() => {
          followBottomFrame = null;
          followBottom();
        });
      }
    };

    const disconnect = observeElementBlockSize(composerEl, applyPadding);

    return () => {
      disconnect();
      if (followBottomFrame !== null) {
        cancelAnimationFrame(followBottomFrame);
      }
      if (mode === "css-variable") {
        scrollEl.style.removeProperty("--composer-overlay-height");
      } else {
        scrollEl.style.paddingBottom = "";
      }
    };
  }, [scrollContainerRef, composerRef, mode, resetKey]);
}
