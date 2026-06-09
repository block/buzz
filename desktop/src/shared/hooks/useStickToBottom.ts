import { useCallback, useEffect, useRef } from "react";

/**
 * Keeps a scroll container pinned to the bottom as new content arrives,
 * unless the user has scrolled up. Mirrors the "sticky scroll" pattern
 * from goose's MessageTimeline.
 *
 * Attach `ref` to the scrollable container and `onScroll` as its scroll
 * handler. The hook observes DOM mutations inside the container and
 * auto-scrolls when the user is near the bottom (within 100 px).
 *
 * Scroll calls are batched via `requestAnimationFrame` so rapid streaming
 * updates (e.g. token-by-token SSE) don't cause layout thrashing.
 */
export function useStickToBottom<T extends HTMLElement = HTMLDivElement>() {
  const ref = useRef<T>(null);
  const isNearBottomRef = useRef(true);

  const onScroll = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    isNearBottomRef.current = scrollHeight - scrollTop - clientHeight < 100;
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // Start at the bottom; the observer below only reacts to later changes.
    el.scrollTop = el.scrollHeight;

    let rafId: number | null = null;

    const scrollIfSticky = () => {
      // Coalesce to one scroll per animation frame.
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        if (isNearBottomRef.current && ref.current) {
          ref.current.scrollTo({
            top: ref.current.scrollHeight,
            behavior: "smooth",
          });
        }
      });
    };

    const observer = new MutationObserver(scrollIfSticky);
    observer.observe(el, {
      childList: true,
      subtree: true,
      characterData: true,
    });

    return () => {
      observer.disconnect();
      if (rafId !== null) cancelAnimationFrame(rafId);
    };
  }, []);

  return { ref, onScroll, isNearBottomRef };
}
