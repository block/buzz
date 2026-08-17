import * as React from "react";

export const MODAL_EXIT_ANIMATION_MS = 150;

export function useDeferredModalOpen() {
  const frameRef = React.useRef<number | null>(null);
  const timeoutRef = React.useRef<number | null>(null);

  const cancelDeferredModalOpen = React.useCallback(() => {
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }

    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const openNextFrame = React.useCallback(
    (open: () => void) => {
      cancelDeferredModalOpen();
      frameRef.current = window.requestAnimationFrame(() => {
        frameRef.current = null;
        open();
      });
    },
    [cancelDeferredModalOpen],
  );

  const openAfterExit = React.useCallback(
    (open: () => void) => {
      cancelDeferredModalOpen();
      timeoutRef.current = window.setTimeout(() => {
        timeoutRef.current = null;
        openNextFrame(open);
      }, MODAL_EXIT_ANIMATION_MS);
    },
    [cancelDeferredModalOpen, openNextFrame],
  );

  React.useEffect(() => cancelDeferredModalOpen, [cancelDeferredModalOpen]);

  return {
    cancelDeferredModalOpen,
    openAfterExit,
    openNextFrame,
  };
}
