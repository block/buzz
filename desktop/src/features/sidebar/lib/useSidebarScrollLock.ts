import { useRouter } from "@tanstack/react-router";
import * as React from "react";

/**
 * Prevents TanStack Router's scroll restoration from moving the sidebar.
 *
 * The router registers a document-level capture listener for "scroll" that
 * records every scrollable element's position. On navigation it restores those
 * positions synchronously inside an "onRendered" event. We snapshot the
 * sidebar's scrollTop in "onBeforeLoad" (before any restoration happens) and
 * re-apply it in "onRendered" (after the router's restoration subscriber has
 * already run, since our subscription is registered later).
 */
export function useSidebarScrollLock(
  scrollRef: React.RefObject<HTMLDivElement | null>,
) {
  const savedScrollTop = React.useRef(0);
  const router = useRouter();

  React.useEffect(() => {
    const unsubBefore = router.subscribe("onBeforeLoad", () => {
      const el = scrollRef.current;
      if (el) {
        savedScrollTop.current = el.scrollTop;
      }
    });

    const unsubRendered = router.subscribe("onRendered", () => {
      const el = scrollRef.current;
      if (el && el.scrollTop !== savedScrollTop.current) {
        el.scrollTop = savedScrollTop.current;
      }
    });

    return () => {
      unsubBefore();
      unsubRendered();
    };
  }, [router, scrollRef]);

  React.useEffect(() => {
    const scrollElement = scrollRef.current;
    if (!scrollElement) return;

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;

      const maxScrollTop =
        scrollElement.scrollHeight - scrollElement.clientHeight;
      const atTop = scrollElement.scrollTop <= 0;
      const atBottom = scrollElement.scrollTop >= maxScrollTop - 1;
      const scrollingPastTop = event.deltaY < 0 && atTop;
      const scrollingPastBottom = event.deltaY > 0 && atBottom;
      if (maxScrollTop > 0 && !scrollingPastTop && !scrollingPastBottom) return;

      event.preventDefault();
      event.stopPropagation();
      scrollElement.scrollTop = scrollingPastTop ? 0 : maxScrollTop;
    };

    scrollElement.addEventListener("wheel", handleWheel, {
      capture: true,
      passive: false,
    });
    return () => {
      scrollElement.removeEventListener("wheel", handleWheel, {
        capture: true,
      });
    };
  }, [scrollRef]);
}
