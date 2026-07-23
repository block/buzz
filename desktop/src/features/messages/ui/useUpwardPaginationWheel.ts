import * as React from "react";

export function useUpwardPaginationWheel(
  hostRef: React.RefObject<HTMLDivElement | null>,
  onWheel: () => void,
) {
  const suppressRef = React.useRef(false);
  const lastUpwardWheelAtRef = React.useRef(Number.NEGATIVE_INFINITY);
  const clear = React.useCallback(() => {
    suppressRef.current = false;
  }, []);

  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    let releaseTimer: number | null = null;
    const handleWheel = (event: WheelEvent) => {
      onWheel();
      if (event.deltaY >= 0) {
        clear();
        if (releaseTimer !== null) window.clearTimeout(releaseTimer);
        releaseTimer = null;
        return;
      }
      lastUpwardWheelAtRef.current = performance.now();
      if (!suppressRef.current) return;
      event.preventDefault();
      if (releaseTimer !== null) window.clearTimeout(releaseTimer);
      releaseTimer = window.setTimeout(() => {
        clear();
        releaseTimer = null;
      }, 80);
    };
    scroller.addEventListener("wheel", handleWheel, { passive: false });
    return () => {
      scroller.removeEventListener("wheel", handleWheel);
      if (releaseTimer !== null) window.clearTimeout(releaseTimer);
    };
  }, [clear, hostRef, onWheel]);

  const arm = React.useCallback(
    (startedPaging: boolean) => {
      const scroller = hostRef.current?.firstElementChild;
      if (
        startedPaging &&
        scroller instanceof HTMLDivElement &&
        scroller.scrollHeight - scroller.clientHeight > 400 &&
        performance.now() - lastUpwardWheelAtRef.current < 120
      ) {
        suppressRef.current = true;
      }
    },
    [hostRef],
  );

  return { arm, clear };
}
