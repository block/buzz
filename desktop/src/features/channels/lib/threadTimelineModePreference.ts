import * as React from "react";

/**
 * Device-level preference for how clicking a channel thread summary presents
 * replies.
 *
 * - `panel` keeps the existing split/focus thread panel behavior.
 * - `inline` expands the currently selected thread inside the channel
 *   timeline while preserving the panel as an explicit backlink.
 */
export type ThreadTimelineMode = "inline" | "panel";

const STORAGE_KEY = "buzz.channels.threadTimelineMode";
const DEFAULT_THREAD_TIMELINE_MODE: ThreadTimelineMode = "panel";

const listeners = new Set<() => void>();

let threadTimelineMode = readStoredThreadTimelineMode();

function parseThreadTimelineMode(
  value: string | null | undefined,
): ThreadTimelineMode {
  return value === "inline" || value === "panel"
    ? value
    : DEFAULT_THREAD_TIMELINE_MODE;
}

function readStoredThreadTimelineMode(): ThreadTimelineMode {
  try {
    return parseThreadTimelineMode(
      globalThis.localStorage?.getItem(STORAGE_KEY),
    );
  } catch {
    return DEFAULT_THREAD_TIMELINE_MODE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ThreadTimelineMode {
  return threadTimelineMode;
}

function getServerSnapshot(): ThreadTimelineMode {
  return DEFAULT_THREAD_TIMELINE_MODE;
}

export function getThreadTimelineMode(): ThreadTimelineMode {
  return threadTimelineMode;
}

export function setThreadTimelineMode(mode: ThreadTimelineMode): void {
  threadTimelineMode = mode;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, mode);
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

export function useThreadTimelineMode(): ThreadTimelineMode {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
