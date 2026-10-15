import * as React from "react";

export type VirtualizedViewportSize = { width: number; height: number };

export function shouldSettleVirtualizedViewportResize({
  virtualizerAtBottom,
  previousSize,
  nextSize,
}: {
  virtualizerAtBottom: boolean;
  /** Last size this observer acted on, or null before the first delivery. */
  previousSize?: VirtualizedViewportSize | null;
  nextSize?: VirtualizedViewportSize | null;
}): boolean {
  if (!virtualizerAtBottom) return false;
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
  const borderBox = entry.borderBoxSize?.[0];
  if (borderBox) {
    return { width: borderBox.inlineSize, height: borderBox.blockSize };
  }
  const rect = entry.contentRect;
  if (!rect) return null;
  return { width: rect.width, height: rect.height };
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
  settleAtBottom,
  requestFrame,
  cancelFrame,
}: {
  virtualizerAtBottomRef: { current: boolean };
  settleAtBottom: () => void;
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
        previousSize,
        nextSize,
      });
      if (nextSize) previousSize = nextSize;
      if (!settle || frame !== null) return;
      frame = requestFrame(() => {
        frame = null;
        settleAtBottom();
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

/** Re-settles a bottom-pinned virtualized timeline after viewport reflow. */
export function useVirtualizedViewportResize(
  scrollContainerRef: React.RefObject<HTMLDivElement | null>,
  virtualizerAtBottomRef: React.RefObject<boolean>,
  settleAtBottom?: () => void,
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
      settleAtBottom,
      requestFrame: requestAnimationFrame,
      cancelFrame: cancelAnimationFrame,
    });
    const observer = new ResizeObserver(handler.handleEntries);
    observer.observe(container);
    return () => {
      observer.disconnect();
      handler.cancel();
    };
  }, [scrollContainerRef, settleAtBottom, virtualizerAtBottomRef]);
}
