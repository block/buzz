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
}
