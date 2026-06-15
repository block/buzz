import * as React from "react";

/**
 * Returns `Date.now()`, re-rendering the calling component every `intervalMs`.
 * Each consumer owns one `setInterval` cleaned up on unmount — mount the hook
 * only where a live clock is actually displayed so idle components never tick.
 */
export function useNow(intervalMs: number): number {
  const [now, setNow] = React.useState(() => Date.now());

  React.useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);

  return now;
}
