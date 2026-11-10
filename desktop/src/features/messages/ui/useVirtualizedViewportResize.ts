import * as React from "react";

export function shouldSettleVirtualizedViewportResize({
  targetHeld,
  virtualizerAtBottom,
}: {
  targetHeld: boolean;
  virtualizerAtBottom: boolean;
}): boolean {
  return virtualizerAtBottom && !targetHeld;
}

/** Re-settles a bottom-pinned virtualized timeline after viewport reflow.
 *  While an accepted navigation target holds the viewport, it re-anchors
 *  that target instead. */
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

    const observer = new ResizeObserver(() => {
      if (
        shouldSettleVirtualizedViewportResize({
          targetHeld: targetHoldRef.current,
          virtualizerAtBottom: virtualizerAtBottomRef.current,
        })
      ) {
        settleAtBottom();
      } else if (targetHoldRef.current) {
        reanchorHeldTarget?.();
      }
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [
    reanchorHeldTarget,
    scrollContainerRef,
    settleAtBottom,
    targetHoldRef,
    virtualizerAtBottomRef,
  ]);
}
