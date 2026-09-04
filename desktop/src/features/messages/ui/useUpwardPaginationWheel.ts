import * as React from "react";

export function useUpwardPaginationWheel(
  hostRef: React.RefObject<HTMLDivElement | null>,
  onWheel: () => void,
) {
  const suppressUntilRef = React.useRef(Number.NEGATIVE_INFINITY);
  const lastUpwardWheelAtRef = React.useRef(Number.NEGATIVE_INFINITY);
  const clear = React.useCallback(() => {
    suppressUntilRef.current = Number.NEGATIVE_INFINITY;
  }, []);

  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    const handleWheel = (event: WheelEvent) => {
      // Ctrl+wheel belongs to browser zoom. It must not retire bottom intent or
      // arm upward-pagination momentum because it does not move the reader.
      if (event.ctrlKey) return;
      onWheel();
      if (event.deltaY >= 0) {
        clear();
        return;
      }
      lastUpwardWheelAtRef.current = performance.now();
      if (performance.now() >= suppressUntilRef.current) return;
      event.preventDefault();
      suppressUntilRef.current = performance.now() + 80;
    };
    scroller.addEventListener("wheel", handleWheel, { passive: false });
    return () => {
      scroller.removeEventListener("wheel", handleWheel);
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
        // Expiry starts with the triggering tick, even if it was the last
        // tick of this gesture. No timer or future suppressed event is needed.
        suppressUntilRef.current = performance.now() + 80;
      }
    },
    [hostRef],
  );

  return { arm, clear };
}
