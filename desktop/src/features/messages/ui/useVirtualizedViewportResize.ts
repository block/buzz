import * as React from "react";

export type VirtualizedViewportSize = { width: number; height: number };

export function shouldSettleVirtualizedViewportResize({
  virtualizerAtBottom,
  targetHeld,
  previousSize,
  nextSize,
}: {
  virtualizerAtBottom: boolean;
  targetHeld: boolean;
  /** Last size delivered to this observer, or null before the first delivery. */
  previousSize?: VirtualizedViewportSize | null;
  nextSize?: VirtualizedViewportSize | null;
}): boolean {
  if (!virtualizerAtBottom || targetHeld) return false;
  // No baseline yet (or no readable box): the first delivery still settles, as
  // it always has.
  if (!previousSize || !nextSize) return true;
  // A resize that round-trips to the same geometry has nothing to settle, and
  // settling anyway is what keeps re-entering the observation pass.
  return (
    previousSize.width !== nextSize.width ||
    previousSize.height !== nextSize.height
  );
}

export function readObservedViewportSize(
  entry: ResizeObserverEntry,
): VirtualizedViewportSize | null {
  // observe() uses the content box, which can change when a scrollbar appears.
  const rect = entry.contentRect;
  if (rect) return { width: rect.width, height: rect.height };
  const borderBox = entry.borderBoxSize?.[0];
  return borderBox
    ? { width: borderBox.inlineSize, height: borderBox.blockSize }
    : null;
}

/**
 * The ResizeObserver callback, lifted out of the hook so it can be driven
 * without a DOM.
 *
 * `settleAtBottom` scrolls, which re-measures rows and can toggle the
 * scrollbar — resizing the observed element at a deeper depth of the pass that
 * is delivering to us. Running the write on the next frame keeps it out of that
 * pass, so the browser stops reporting "ResizeObserver loop completed with
 * undelivered notifications". This mirrors the rAF defer the sibling observer in
 * `useVirtualizedBottomSettle` already uses.
 */
export function createVirtualizedViewportResizeHandler({
  virtualizerAtBottomRef,
  targetHoldRef,
  settleAtBottom,
  reanchorHeldTarget,
  requestFrame,
  cancelFrame,
}: {
  virtualizerAtBottomRef: { current: boolean };
  targetHoldRef: { current: boolean };
  settleAtBottom: () => void;
  reanchorHeldTarget?: () => void;
  requestFrame: (cb: () => void) => number;
  cancelFrame: (handle: number) => void;
}) {
  let previousSize: VirtualizedViewportSize | null = null;
  let frame: number | null = null;

  return {
    handleEntries(entries: readonly ResizeObserverEntry[]) {
      const entry = entries.at(-1);
      const nextSize = entry ? readObservedViewportSize(entry) : null;
      const settle = shouldSettleVirtualizedViewportResize({
        virtualizerAtBottom: virtualizerAtBottomRef.current,
        targetHeld: targetHoldRef.current,
        previousSize,
        nextSize,
      });
      if (nextSize) previousSize = nextSize;
      if (frame !== null || (!settle && !targetHoldRef.current)) return;
      frame = requestFrame(() => {
        frame = null;
        // Navigation and reader scroll can change which position we own.
        if (targetHoldRef.current) {
          reanchorHeldTarget?.();
        } else if (virtualizerAtBottomRef.current) {
          settleAtBottom();
        }
      });
    },
    cancel() {
      if (frame !== null) {
        cancelFrame(frame);
        frame = null;
      }
    },
  };
}

/** Re-anchors a held navigation target or bottom-pinned timeline after reflow. */
export function useVirtualizedViewportResize(
  scrollContainerRef: React.RefObject<HTMLDivElement | null>,
  virtualizerAtBottomRef: React.RefObject<boolean>,
  targetHoldRef: React.RefObject<boolean>,
  settleAtBottom?: () => void,
  reanchorHeldTarget?: () => void,
) {
  React.useEffect(() => {
    const container = scrollContainerRef.current;
    if (
      !container ||
      !settleAtBottom ||
      typeof ResizeObserver === "undefined"
    ) {
      return;
    }

    const handler = createVirtualizedViewportResizeHandler({
      virtualizerAtBottomRef,
      targetHoldRef,
      settleAtBottom,
      reanchorHeldTarget,
      requestFrame: requestAnimationFrame,
      cancelFrame: cancelAnimationFrame,
    });
    const observer = new ResizeObserver(handler.handleEntries);
    observer.observe(container);
    return () => {
      observer.disconnect();
      handler.cancel();
    };
  }, [
    reanchorHeldTarget,
    scrollContainerRef,
    settleAtBottom,
    targetHoldRef,
    virtualizerAtBottomRef,
  ]);
}
