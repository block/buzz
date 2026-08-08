import * as React from "react";

const MINUTE_MS = 60_000;
let currentMinute = Date.now();
let minuteTimer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (minuteTimer === null) {
    minuteTimer = setInterval(() => {
      currentMinute = Date.now();
      for (const notify of listeners) notify();
    }, MINUTE_MS);
  }

  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && minuteTimer !== null) {
      clearInterval(minuteTimer);
      minuteTimer = null;
    }
  };
}

function getSnapshot() {
  return currentMinute;
}

/** A shared minute clock for recency labels rendered in large lists. */
export function useMinuteNow(): number {
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
