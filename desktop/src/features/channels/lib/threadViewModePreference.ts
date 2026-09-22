import * as React from "react";

/**
 * Thread open layout inside a channel.
 *
 * HulaBuzz locks this to `split` (side panel next to the channel). The older
 * `focus` drawer mode and its Appearance selector are removed; callers may
 * still mention `"focus"` in types for upstream compatibility, but reads and
 * writes always resolve to `split`.
 */
export type ThreadViewMode = "focus" | "split";

const STORAGE_KEY = "buzz.channels.threadViewMode";

/** HulaBuzz always opens threads in the split pane. */
const DEFAULT_THREAD_VIEW_MODE: ThreadViewMode = "split";

const listeners = new Set<() => void>();

let threadViewMode: ThreadViewMode = DEFAULT_THREAD_VIEW_MODE;

function clearStoredFocusPreference(): void {
  try {
    const stored = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (stored === "focus") {
      globalThis.localStorage?.setItem(STORAGE_KEY, DEFAULT_THREAD_VIEW_MODE);
    }
  } catch {
    // Persistence is best-effort.
  }
}

clearStoredFocusPreference();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ThreadViewMode {
  return threadViewMode;
}

function getServerSnapshot(): ThreadViewMode {
  return DEFAULT_THREAD_VIEW_MODE;
}

/** Read the thread layout preference outside of React. Always `split` in HulaBuzz. */
export function getThreadViewMode(): ThreadViewMode {
  return threadViewMode;
}

/**
 * Update the thread layout preference.
 * HulaBuzz ignores non-split values so Focus cannot be re-enabled.
 */
export function setThreadViewMode(mode: ThreadViewMode): void {
  const next = mode === "split" ? "split" : DEFAULT_THREAD_VIEW_MODE;
  if (next === threadViewMode) {
    // Still rewrite localStorage if a stale focus value is present.
    clearStoredFocusPreference();
    return;
  }

  threadViewMode = next;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, next);
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** How threads open in a channel. HulaBuzz always returns `split`. */
export function useThreadViewMode(): ThreadViewMode {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
