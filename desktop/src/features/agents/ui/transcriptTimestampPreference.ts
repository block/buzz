import * as React from "react";

/**
 * User preference for showing a timestamp under each activity row — polished
 * transcript rows and raw JSON-RPC cards alike. Persisted in localStorage and
 * shared across every transcript surface, same as the animation preference.
 * This is a device-level UI preference, not workspace-scoped data, so it is
 * intentionally not reset on workspace switch. Defaults off to keep the feed
 * compact.
 */
const STORAGE_KEY = "buzz:show-transcript-timestamps";

const listeners = new Set<() => void>();

let timestampsEnabled = readStoredPreference();

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
  return timestampsEnabled;
}

function getServerSnapshot(): boolean {
  return false;
}

/** Update the preference and notify all subscribed components. */
export function setTranscriptTimestampsEnabled(enabled: boolean): void {
  timestampsEnabled = enabled;

  try {
    window.localStorage.setItem(STORAGE_KEY, enabled ? "1" : "0");
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** Whether polished activity rows should show a timestamp footer. */
export function useTranscriptTimestampsEnabled(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
