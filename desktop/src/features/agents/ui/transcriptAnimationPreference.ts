import * as React from "react";

/**
 * User preference for animating transcript activity rows as they stream in.
 * Persisted in localStorage and shared across every transcript surface
 * (thread panel, profile live-activity preview). This is a device-level UI
 * preference, not workspace-scoped data, so it is intentionally not reset on
 * workspace switch.
 */
const STORAGE_KEY = "buzz:animate-transcript-activity";

const listeners = new Set<() => void>();

let animationEnabled = readStoredPreference();

function readStoredPreference(): boolean {
  if (typeof window === "undefined") {
    return true;
  }

  try {
    return window.localStorage.getItem(STORAGE_KEY) !== "0";
  } catch {
    return true;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): boolean {
  return animationEnabled;
}

function getServerSnapshot(): boolean {
  return true;
}

/** Update the preference and notify all subscribed components. */
export function setTranscriptAnimationEnabled(enabled: boolean): void {
  animationEnabled = enabled;

  try {
    window.localStorage.setItem(STORAGE_KEY, enabled ? "1" : "0");
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** Whether transcript activity rows should animate in. */
export function useTranscriptAnimationEnabled(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
