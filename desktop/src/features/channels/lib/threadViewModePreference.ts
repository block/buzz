import * as React from "react";

/**
 * User preference for how a thread opens inside a channel.
 *
 * - `focus` — a large right-anchored drawer overlays the channel content area,
 *   leaving a narrow scrim-dimmed sliver of the channel visible as an
 *   orientation cue and a click-target back to the channel.
 * - `split` — the thread opens in a resizable side panel next to the channel.
 *
 * Persisted in localStorage. This is a device-level UI preference, not
 * community-scoped data, so it is intentionally not reset on community switch.
 * Only applies at viewport widths wide enough for a two-pane channel view;
 * narrow viewports keep their single-panel/floating-overlay behavior.
 */
export type ThreadViewMode = "focus" | "split";

const STORAGE_KEY = "buzz.channels.threadViewMode";

/** Layout used when nothing is stored, or the stored value is unrecognized. */
const DEFAULT_THREAD_VIEW_MODE: ThreadViewMode = "split";

const listeners = new Set<() => void>();

let threadViewMode = readStoredThreadViewMode();

function parseThreadViewMode(value: string | null | undefined): ThreadViewMode {
  return value === "focus" || value === "split"
    ? value
    : DEFAULT_THREAD_VIEW_MODE;
}

function readStoredThreadViewMode(): ThreadViewMode {
  try {
    return parseThreadViewMode(globalThis.localStorage?.getItem(STORAGE_KEY));
  } catch {
    return DEFAULT_THREAD_VIEW_MODE;
  }
}

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

/** Read the persisted thread layout preference outside of React. */
export function getThreadViewMode(): ThreadViewMode {
  return threadViewMode;
}

/** Update the thread layout preference and notify all subscribed components. */
export function setThreadViewMode(mode: ThreadViewMode): void {
  threadViewMode = mode;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, mode);
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** How threads should open in a channel: as a focus drawer or a split pane. */
export function useThreadViewMode(): ThreadViewMode {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
