import * as React from "react";

const DEFAULT_TIMEOUT_MS = 2_000;

/**
 * Defers activation until the browser is idle (via `requestIdleCallback`) or
 * a timeout elapses — whichever comes first.
 *
 * Pass `immediate: true` to skip deferral and return `true` on the first render
 * (useful when the deferred content is already in view).
 */
export function useDeferredLoad(
  options: { immediate?: boolean; timeoutMs?: number } = {},
): boolean {
  const { immediate = false, timeoutMs = DEFAULT_TIMEOUT_MS } = options;
  const [isReady, setIsReady] = React.useState(immediate);

  React.useEffect(() => {
    if (isReady) {
      return;
    }

    if (immediate) {
      setIsReady(true);
      return;
    }

    const activate = () => {
      setIsReady(true);
    };

    if ("requestIdleCallback" in window) {
      const idleId = window.requestIdleCallback(activate, {
        timeout: timeoutMs,
      });
      return () => {
        window.cancelIdleCallback(idleId);
      };
    }

    const timeoutId = globalThis.setTimeout(activate, timeoutMs);
    return () => {
      globalThis.clearTimeout(timeoutId);
    };
  }, [immediate, isReady, timeoutMs]);

  return isReady;
}

let hasCompletedStartup = false;

/**
 * Convenience wrapper: defers non-critical startup work (presence, notifications,
 * subscriptions) until the main shell is interactive. Uses a module-level flag
 * so the deferral only happens once per app lifecycle — remounts skip the delay.
 */
export function useDeferredStartup(): boolean {
  const ready = useDeferredLoad();
  if (ready) {
    hasCompletedStartup = true;
  }
  return hasCompletedStartup || ready;
}
