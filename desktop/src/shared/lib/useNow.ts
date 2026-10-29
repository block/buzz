import * as React from "react";

import { useDocumentVisible } from "@/shared/lib/useDocumentVisible";

type SharedTicker = {
  listeners: Set<() => void>;
  intervalId: ReturnType<typeof setInterval>;
};

const tickers = new Map<number, SharedTicker>();

/**
 * One `setInterval` per distinct interval, shared by every subscriber. All
 * same-cadence consumers tick in a single timer callback, so React batches
 * their state updates into one render/composite pass instead of N unaligned
 * passes per interval. With dozens of "working" badges mounted, per-consumer
 * timers pinned a sustained ~25% of a core in the WebKit process.
 */
function subscribeToSharedTick(
  intervalMs: number,
  listener: () => void,
): () => void {
  let ticker = tickers.get(intervalMs);
  if (!ticker) {
    const created: SharedTicker = {
      listeners: new Set(),
      intervalId: setInterval(() => {
        for (const tick of created.listeners) tick();
      }, intervalMs),
    };
    tickers.set(intervalMs, created);
    ticker = created;
  }
  ticker.listeners.add(listener);

  return () => {
    ticker.listeners.delete(listener);
    if (ticker.listeners.size === 0) {
      clearInterval(ticker.intervalId);
      tickers.delete(intervalMs);
    }
  };
}

/**
 * Returns `Date.now()`, re-rendering the calling component every `intervalMs`.
 * Consumers with the same interval share one timer — mount the hook only where
 * a live clock is actually displayed so idle components never tick.
 */
export function useNow(intervalMs: number): number {
  const [now, setNow] = React.useState(() => Date.now());
  const documentVisible = useDocumentVisible();

  React.useEffect(() => {
    if (!documentVisible) return;

    setNow(Date.now());
    return subscribeToSharedTick(intervalMs, () => setNow(Date.now()));
  }, [documentVisible, intervalMs]);

  return now;
}
