import * as React from "react";

import { CONVERGENCE_FRAME_CAP } from "@/features/messages/lib/scrollConvergence";
import type { ListVirtualizer } from "@/shared/ui/VirtualizedList";

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  restoreScrollPosition: (scrollTop: number) => void;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  sentinelRef: React.RefObject<HTMLDivElement | null>;
  /**
   * When the timeline is virtualized, a prepend shifts every index and a large
   * one pushes the anchored row out of the window before it can be re-measured.
   * Supplying the virtualizer switches to an index anchor: we hold the
   * first-visible row across the prepend by re-aiming `scrollToIndex` at its new
   * index (resolved from `indexByMessageId`) until the library settles it.
   */
  virtualizer?: {
    getVirtualizer: () => ListVirtualizer | null;
    indexByMessageId: Map<string, number>;
    itemCount: number;
  } | null;
};

/**
 * Triggers `fetchOlder` when a sentinel element near the top of the scroll
 * container enters the viewport, then restores the scroll position so the
 * visible content doesn't jump.
 */
export function useLoadOlderOnScroll({
  fetchOlder,
  hasOlderMessages,
  isLoading,
  restoreScrollPosition,
  scrollContainerRef,
  sentinelRef,
  virtualizer = null,
}: UseLoadOlderOnScrollOptions) {
  const restoreScrollPositionRef = React.useRef(restoreScrollPosition);
  React.useEffect(() => {
    restoreScrollPositionRef.current = restoreScrollPosition;
  });
  // Mirror the virtualizer option into a ref so the long-lived Intersection
  // observer reads the live getter + count without re-subscribing per render.
  const virtualizerRef = React.useRef(virtualizer);
  virtualizerRef.current = virtualizer;

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

          const virt = virtualizerRef.current;
          if (virt) {
            // Hold the first VISIBLE row across the prepend. After N older rows
            // are prepended the anchored row's INDEX shifts by N and — with
            // scrollTop unchanged near the top — it's pushed below the window
            // and recycled out of the DOM, so a pure DOM re-read can't find it.
            // We therefore drive the virtualizer: capture the row's id + its top
            // offset in the viewport now, and after the prepend re-aim
            // `scrollToIndex(newIndex, "start")` each frame (re-issued only when
            // the resolved index moves, so the library's internal settle loop is
            // never reset — same single-issue discipline as the convergence
            // adapter). `scrollToIndex` re-aims internally as rows mount and
            // measure, landing the row's TOP at the viewport top; once it settles
            // we apply the captured intra-viewport gap with ONE scrollTop write.
            // Single writer throughout: one mechanism re-aims, the gap is a final
            // one-shot, never an overlapping second target.
            const instance = virt.getVirtualizer();
            const container = scrollContainerRef.current;
            const previousCount = virt.itemCount;

            void fetchOlder().then(() => {
              // Capture the anchor at fetch-RESOLVE time, not sentinel-fire
              // time: the history request can be in flight for a while and the
              // user may keep scrolling during it (the e2e scrolls further down
              // mid-fetch). The row+offset we must preserve is wherever the
              // reader actually is the instant before the prepend commits, read
              // from the live DOM here while every visible row is still mounted.
              const containerRect = container?.getBoundingClientRect();
              const containerTop = containerRect?.top ?? 0;
              const containerBottom = containerRect?.bottom ?? 0;
              // First row intersecting the viewport — the reader's eye-line row.
              // Geometry matches the test's getFirstVisibleMessage exactly: its
              // bottom is below the viewport top and its top is above the
              // viewport bottom.
              const anchorRow = container
                ? Array.from(
                    container.querySelectorAll<HTMLElement>(
                      "[data-message-id]",
                    ),
                  ).find((row) => {
                    const rect = row.getBoundingClientRect();
                    return (
                      rect.bottom > containerTop && rect.top < containerBottom
                    );
                  })
                : undefined;
              const anchorId = anchorRow?.dataset.messageId ?? null;
              // The anchored row's top relative to the viewport top — held
              // constant across the prepend.
              const anchorTop = anchorRow
                ? anchorRow.getBoundingClientRect().top - containerTop
                : 0;

              // The timeline drives its rows off a `useDeferredValue` of the
              // message list, so the prepended items commit on a LOW-priority
              // render that can land several frames after `fetchOlder` resolves.
              // Poll rAF until the live id->index map actually shifts the anchor
              // (the prepend is observable), capped so an empty fetch can't spin.
              const maxFrames = CONVERGENCE_FRAME_CAP;
              let frame = 0;
              let lastTarget: number | null = null;
              let stableFrames = 0;
              const waitForPrepend = () => {
                const after = virtualizerRef.current;
                const grew =
                  (after?.itemCount ?? previousCount) > previousCount;
                const newIndex =
                  anchorId !== null
                    ? after?.indexByMessageId.get(anchorId)
                    : undefined;
                if (instance && grew && newIndex !== undefined) {
                  // Target offset that puts the anchored row back at its captured
                  // viewport gap: the row's start (top at viewport top) minus the
                  // gap that was above it. We drive `scrollToOffset` — NOT
                  // `scrollToIndex` — so the library's reconcile holds a FIXED
                  // offset (`scrollState.index` is null) instead of re-resolving
                  // the index to row-top each frame and overwriting our gap. As
                  // the prepended rows measure, `getOffsetForIndex` grows and we
                  // recompute the target and re-issue. Re-issue ONLY when the
                  // target moves — re-issuing an unchanged offset resets the
                  // library's stable-frame counter and spins. Same single-issue
                  // discipline as the convergence adapter; one mechanism.
                  const start = instance.getOffsetForIndex(newIndex, "start");
                  if (start !== undefined) {
                    const target = start[0] - anchorTop;
                    if (target !== lastTarget) {
                      instance.scrollToOffset(target, { align: "start" });
                      lastTarget = target;
                      stableFrames = 0;
                    } else if ((instance.scrollOffset ?? 0) === target) {
                      // The offset reached its target and the target stopped
                      // moving (measurement settled). Two stable frames guards
                      // against ending before the last row measures.
                      stableFrames += 1;
                      if (stableFrames >= 2) {
                        observe();
                        return;
                      }
                    }
                  }
                }
                frame += 1;
                if (frame >= maxFrames) {
                  observe();
                  return;
                }
                requestAnimationFrame(waitForPrepend);
              };
              requestAnimationFrame(waitForPrepend);
            });
            return;
          }

          const previousHeight = container.scrollHeight;
          const previousScrollTop = container.scrollTop;
          void fetchOlder().then(() => {
            requestAnimationFrame(() => {
              requestAnimationFrame(() => {
                const newHeight = container.scrollHeight;
                const delta = newHeight - previousHeight;
                if (delta > 0) {
                  restoreScrollPositionRef.current(previousScrollTop + delta);
                }
                observe();
              });
            });
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
