import * as React from "react";

/**
 * Device-level preference for showing agent activity rows inline in Bestie.
 * Kept for a later Bestie DM feature; the popover no longer renders activity.
 * Persisted in localStorage; defaults OFF.
 */
const STORAGE_KEY = "buzz-bestie-show-activity.v1";

const listeners = new Set<() => void>();

let showActivity = readStoredPreference();

function readStoredPreference(): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  try {
    return window.localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): boolean {
  return showActivity;
}

function getServerSnapshot(): boolean {
  return false;
}

/** Update the preference and notify all subscribed components. */
export function setBestieShowActivity(enabled: boolean): void {
  showActivity = enabled;

  try {
    window.localStorage.setItem(STORAGE_KEY, enabled ? "1" : "0");
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** Whether Bestie should render inline agent activity rows (DM feature later). */
export function useBestieShowActivity(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export const BESTIE_SHOW_ACTIVITY_STORAGE_KEY = STORAGE_KEY;
