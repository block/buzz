import * as React from "react";

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  sentinelRef: React.RefObject<HTMLDivElement | null>;
};

// How far above the viewport top the sentinel triggers a fetch. Larger = the
// next page preloads sooner as you scroll up. Kept comfortably below a typical
// prepend height (one fetch lands >=30 rows) so the landed page still pushes
// the sentinel out of the band and re-arms the once-per-gesture gate; a band
// taller than the prepend would leave the sentinel inside it and stall paging.
const PRELOAD_MARGIN_PX = 600;

/**
 * Triggers `fetchOlder` when a sentinel element near the top of the scroll
 * container enters the viewport.
 *
 * A single long-lived observer drives this: it fires `fetchOlder` once when the
 * sentinel enters the trigger band, then *disarms* and will not fire again until
 * the sentinel has left the band and re-entered it — i.e. a fresh scroll-up
 * gesture. Re-arming with a new observer the instant the fetch resolved (the
 * previous approach) fired a second fetch immediately: the prepended rows commit
 * on a deferred snapshot a few frames later, so the sentinel is still inside the
 * band when the new observer first reports it, cascading page-after-page off one
 * gesture. Gating on the leave→enter transition needs no scroll-position
 * coupling and pages exactly once per gesture.
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
    // Armed when the sentinel is outside the band; a single entry into the band
    // fires one fetch and disarms until the sentinel leaves again.
    let armed = true;

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (disposed) {
          return;
        }
        if (!entry.isIntersecting) {
          // Sentinel left the band (the prepend pushed it down, or the user
          // scrolled away). Re-arm so the next scroll-up pages once more.
          armed = true;
          return;
        }
        if (!armed) {
          return;
        }
        armed = false;
        void fetchOlder();
      },
      { root: container, rootMargin: `${PRELOAD_MARGIN_PX}px 0px 0px 0px` },
    );

    observer.observe(sentinel);
    return () => {
      disposed = true;
      observer.disconnect();
    };
  }, [
    fetchOlder,
    hasOlderMessages,
    isLoading,
    scrollContainerRef,
    sentinelRef,
  ]);
}
