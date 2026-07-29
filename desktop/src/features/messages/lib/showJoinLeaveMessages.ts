import * as React from "react";

/**
 * Device-local "Show join and leave messages" preference. Hidden by default:
 * channel timelines omit joined/added/left/removed system rows unless the
 * user enables them in Settings. Purely client-side — the relay always
 * delivers the kind:40099 membership events (member lists depend on them);
 * this setting only controls whether they render in the timeline.
 */
const STORAGE_KEY = "buzz:show-join-leave-messages";

const listeners = new Set<() => void>();
let enabled = readEnabled();

function readEnabled(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

export function setShowJoinLeaveMessagesEnabled(next: boolean): void {
  if (enabled === next) return;
  enabled = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next ? "1" : "0");
  } catch {
    // Persistence is best-effort; the live session still uses in-memory state.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): boolean {
  return enabled;
}

export function useShowJoinLeaveMessages(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, () => false);
}
