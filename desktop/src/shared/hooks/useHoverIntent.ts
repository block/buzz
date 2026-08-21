import * as React from "react";

/**
 * Dwell before a hover counts as intent. Long enough that scrubbing the
 * pointer across the sidebar never fires, short enough that a deliberate
 * hover warms the destination well before the click lands.
 */
const HOVER_INTENT_DWELL_MS = 100;

type TimerHost = {
  setTimeout: (fn: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
};

/**
 * Pure dwell-timer core behind {@link useHoverIntent}; injectable timers for
 * unit testing. `start` restarts the dwell; `cancel` drops it.
 */
export function createHoverIntent(
  onIntent: () => void,
  timers: TimerHost,
  dwellMs: number = HOVER_INTENT_DWELL_MS,
): { start: () => void; cancel: () => void } {
  let timerId: number | null = null;
  const cancel = () => {
    if (timerId !== null) {
      timers.clearTimeout(timerId);
      timerId = null;
    }
  };
  return {
    start: () => {
      cancel();
      timerId = timers.setTimeout(() => {
        timerId = null;
        onIntent();
      }, dwellMs);
    },
    cancel,
  };
}

/**
 * Fires `onIntent` after the pointer dwells on an element. Returns stable
 * mouse-enter/leave handlers; the latest callback is always used, and any
 * pending dwell is dropped on unmount.
 */
export function useHoverIntent(onIntent: () => void): {
  onMouseEnter: () => void;
  onMouseLeave: () => void;
} {
  const callbackRef = React.useRef(onIntent);
  callbackRef.current = onIntent;
  const intentRef = React.useRef<ReturnType<typeof createHoverIntent> | null>(
    null,
  );
  if (intentRef.current === null) {
    intentRef.current = createHoverIntent(() => callbackRef.current(), {
      setTimeout: (fn, ms) => window.setTimeout(fn, ms),
      clearTimeout: (id) => window.clearTimeout(id),
    });
  }
  React.useEffect(() => () => intentRef.current?.cancel(), []);
  return {
    onMouseEnter: intentRef.current.start,
    onMouseLeave: intentRef.current.cancel,
  };
}
