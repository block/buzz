import * as React from "react";

import { CONVERGENCE_FRAME_CAP } from "@/features/messages/lib/scrollConvergence";
import type { ListVirtualizer } from "@/shared/ui/VirtualizedList";

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  restoreScrollPosition: (scrollTop: number) => void;
  /**
   * Brackets the index-restore loop's scroll ownership so the anchored hook's
   * ResizeObserver cedes for the duration of the prepend. Without it, the
   * prepended rows measuring late fire the observer with the windowed-out
   * anchor and its all-gone fallback pins the view to the floor, stomping this
   * loop's correct re-aim. Only the virtualized (index) path needs it — the
   * non-virtualized path's restore is a single synchronous `scrollTop` write.
   */
  setLoadOlderRestoreInFlight?: (inFlight: boolean) => void;
  /**
   * Live read of whether the user is stuck-to-bottom. If the user abandons the
   * fetch by jumping to bottom WHILE the index restore owns scroll, the restore
   * loop must re-aim at the bottom rather than the captured mid-history anchor
   * — otherwise it lands the old anchor offset (short of the true floor) and the
   * ceded ResizeObserver never chases the last rows down. Only meaningful with a
   * virtualizer; the non-virtualized path restores synchronously.
   */
  getAnchorIsAtBottom?: () => boolean;
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
    /** Live (non-deferred) message count for growth detection during load-older. */
    liveMessageCount?: number;
  } | null;
};

/**
 * The scroll offset the prepend-restore loop should hold this frame. Returns
 * `undefined` when there's nothing to aim at (no virtualizer, no growth yet, or
 * the captured anchor row isn't resolvable). Resolved off the virtualizer's
 * live measurement each frame so the loop chases the offset as prepended rows
 * grow `getTotalSize()`.
 *   - `abandonedToBottom`: the user jumped to bottom mid-prepend → the last
 *     row's END offset (the true floor), not the stale mid-history anchor.
 *   - otherwise: the captured first-visible row's START offset minus the gap
 *     that was above it, holding the reader's eye-line across the prepend.
 */
function resolveTarget({
  instance,
  abandonedToBottom,
  lastIndex,
  newIndex,
  anchorTop,
}: {
  instance: ListVirtualizer | null;
  abandonedToBottom: boolean;
  lastIndex: number;
  newIndex: number | undefined;
  anchorTop: number;
}): number | undefined {
  if (!instance) return undefined;
  if (abandonedToBottom) {
    if (lastIndex < 0) return undefined;
    return instance.getOffsetForIndex(lastIndex, "end")?.[0];
  }
  if (newIndex === undefined) return undefined;
  const start = instance.getOffsetForIndex(newIndex, "start");
  return start === undefined ? undefined : start[0] - anchorTop;
}

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
  setLoadOlderRestoreInFlight,
  getAnchorIsAtBottom,
  scrollContainerRef,
  sentinelRef,
  virtualizer = null,
}: UseLoadOlderOnScrollOptions) {
  const restoreScrollPositionRef = React.useRef(restoreScrollPosition);
  React.useEffect(() => {
    restoreScrollPositionRef.current = restoreScrollPosition;
  });
  // Mirror the cede setter so the long-lived Intersection observer reads the
  // live callback without re-subscribing (same rationale as the restore ref).
  const setInFlightRef = React.useRef(setLoadOlderRestoreInFlight);
  React.useEffect(() => {
    setInFlightRef.current = setLoadOlderRestoreInFlight;
  });
  // Mirror the at-bottom getter so the prepend loop reads the live abandon
  // state without re-subscribing the long-lived observer per render.
  const getAnchorIsAtBottomRef = React.useRef(getAnchorIsAtBottom);
  React.useEffect(() => {
    getAnchorIsAtBottomRef.current = getAnchorIsAtBottom;
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
            // Use liveMessageCount (live, non-deferred) rather than itemCount
            // or instance.options.count — both are derived from deferredMessages
            // and are stale during a fetch, so the growth check would never fire.
            const previousCount = virt.liveMessageCount ?? virt.itemCount;

            void fetchOlder().then(() => {
              // Claim scroll ownership for the whole re-aim window so the
              // anchored hook's ResizeObserver cedes while the prepended rows
              // measure (released at every exit below via `finishPrepend`).
              setInFlightRef.current?.(true);
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
              // Release scroll ownership (re-enabling the ResizeObserver) and
              // re-arm the sentinel observer. Called at both loop exits.
              const finishPrepend = () => {
                setInFlightRef.current?.(false);
                observe();
              };
              const waitForPrepend = () => {
                const after = virtualizerRef.current;
                const grew =
                  (after?.liveMessageCount ?? previousCount) > previousCount;
                // Resolve this frame's target offset. Two cases, one mechanism:
                //   - Abandon: the user jumped to bottom while this loop owned
                //     scroll. Hold the BOTTOM (last row's end offset), not the
                //     captured mid-history anchor — that old offset sits short of
                //     the true floor and would strand the view there, since the
                //     ResizeObserver re-pin is ceded to this loop for the whole
                //     window.
                //   - Normal: hold the captured first-visible row at its viewport
                //     gap (its start offset minus the gap that was above it).
                // Either way we drive `scrollToOffset` — NOT `scrollToIndex` — so
                // the library's reconcile holds a FIXED offset instead of
                // re-resolving each frame and overwriting it. As the prepended
                // rows measure, `getOffsetForIndex` grows and we recompute and
                // re-issue. Re-issue ONLY when the target moves — re-issuing an
                // unchanged offset resets the library's stable-frame counter and
                // spins. Same single-issue discipline as the convergence adapter.
                const abandonedToBottom =
                  getAnchorIsAtBottomRef.current?.() ?? false;
                const newIndex =
                  anchorId !== null
                    ? after?.indexByMessageId.get(anchorId)
                    : undefined;
                const target = resolveTarget({
                  instance: grew ? instance : null,
                  abandonedToBottom,
                  lastIndex:
                    (after?.liveMessageCount ?? previousCount) - 1,
                  newIndex,
                  anchorTop,
                });
                if (instance && target !== undefined) {
                  if (target !== lastTarget) {
                    instance.scrollToOffset(target, { align: "start" });
                    lastTarget = target;
                    stableFrames = 0;
                  } else if ((instance.scrollOffset ?? 0) >= target) {
                    // Reached the target and it stopped moving (measurement
                    // settled). `>=` not `===`: the abandon path aims at the
                    // last possible offset, which the container clamps, so the
                    // realized `scrollOffset` can sit at-or-past it. Two stable
                    // frames guard against ending before the last row measures.
                    stableFrames += 1;
                    if (stableFrames >= 2) {
                      finishPrepend();
                      return;
                    }
                  }
                }
                frame += 1;
                if (frame >= maxFrames) {
                  finishPrepend();
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
