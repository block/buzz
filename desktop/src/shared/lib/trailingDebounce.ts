export type TrailingDebounce = {
  /** Schedule the action, restarting the quiet window on every call. */
  trigger: () => void;
  /** Cancel any pending action (e.g. on unmount). */
  cancel: () => void;
};

type TimerHost = {
  setTimeout: (handler: () => void, ms: number) => number;
  clearTimeout: (id: number) => void;
};

/**
 * Trailing debounce: a burst of `trigger()` calls collapses into a single
 * `action()` that runs once, `delayMs` after the last call. The timer is
 * restarted on every trigger, so a sustained burst keeps deferring until quiet.
 */
export function createTrailingDebounce(
  action: () => void,
  delayMs: number,
  host: TimerHost = window,
): TrailingDebounce {
  let timeoutId: number | undefined;
  const cancel = () => {
    if (timeoutId !== undefined) {
      host.clearTimeout(timeoutId);
      timeoutId = undefined;
    }
  };
  return {
    trigger: () => {
      cancel();
      timeoutId = host.setTimeout(() => {
        timeoutId = undefined;
        action();
      }, delayMs);
    },
    cancel,
  };
}
