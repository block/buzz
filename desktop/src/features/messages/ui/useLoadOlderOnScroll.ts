import * as React from "react";

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  sentinelRef: React.RefObject<HTMLDivElement | null>;
};

/**
 * Triggers `fetchOlder` when a sentinel element near the top of the scroll
 * container enters the viewport. Native scroll anchoring (overflow-anchor)
 * holds the reading position across the prepend — no JS scroll restore.
 */
export function useLoadOlderOnScroll({
  fetchOlder,
  hasOlderMessages,
  isLoading,
  scrollContainerRef,
  sentinelRef,
}: UseLoadOlderOnScrollOptions) {
  React.useEffect(() => {
    const sentinel = sentinelRef.current;
    const container = scrollContainerRef.current;
    if (
      !sentinel ||
      !container ||
      !fetchOlder ||
      isLoading ||
      !hasOlderMessages
    ) {
      return;
    }

    let disposed = false;
    let currentObserver: IntersectionObserver | null = null;

    const observe = () => {
      if (disposed) {
        return;
      }

      currentObserver = new IntersectionObserver(
        ([entry]) => {
          if (!entry.isIntersecting || disposed) {
            return;
          }

          currentObserver?.disconnect();

          // Native scroll anchoring (overflow-anchor) holds the reading
          // position across the prepend now that all rows stay in the DOM, so
          // the sentinel's only job is to trigger the fetch and re-arm.
          void fetchOlder().then(() => {
            observe();
          });
        },
        { root: container, rootMargin: "200px 0px 0px 0px" },
      );

      currentObserver.observe(sentinel);
    };

    observe();
    return () => {
      disposed = true;
      currentObserver?.disconnect();
    };
  }, [
    fetchOlder,
    hasOlderMessages,
    isLoading,
    scrollContainerRef,
    sentinelRef,
  ]);
}
