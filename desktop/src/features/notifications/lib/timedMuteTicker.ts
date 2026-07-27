import * as React from "react";

/**
 * Single UI-refresh timer for timed mutes.
 *
 * Timed-mute expiry itself is evaluated lazily (`resolveChannelNotifyState`
 * compares `muteUntil` against the current clock), so this module exists only to
 * re-render at the moment the nearest mute expires. One module-level timer
 * serves every consumer; callers schedule the nearest expiry and read the
 * version via `useTimedMuteVersion`.
 */

// setTimeout delays above ~24.8 days overflow and fire immediately; cap the
// wait and let the re-scheduled tick carry us the rest of the way.
const MAX_DELAY_MS = 6 * 60 * 60 * 1_000;
// Small skew so the resolver sees `muteUntil` as strictly past when we fire.
const EXPIRY_SKEW_MS = 250;

let version = 0;
let timer: number | null = null;
let scheduledFor: number | null = null;
const listeners = new Set<() => void>();

function clearTimer(): void {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
  scheduledFor = null;
}

export function getTimedMuteVersion(): number {
  return version;
}

export function subscribeTimedMuteVersion(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Arm (or disarm, with `null`) the refresh timer for the nearest running
 * timed-mute expiry, in Unix seconds. Re-arming for the same instant is a no-op.
 */
export function scheduleTimedMuteRefresh(expirySeconds: number | null): void {
  if (expirySeconds === null) {
    clearTimer();
    return;
  }
  if (scheduledFor === expirySeconds && timer !== null) return;
  clearTimer();
  const delay = Math.min(
    MAX_DELAY_MS,
    Math.max(0, expirySeconds * 1_000 - Date.now() + EXPIRY_SKEW_MS),
  );
  scheduledFor = expirySeconds;
  timer = window.setTimeout(() => {
    timer = null;
    scheduledFor = null;
    version += 1;
    for (const listener of listeners) listener();
  }, delay);
}

/** Community-scoped teardown — see resetCommunityState() in useCommunityInit. */
export function resetTimedMuteTicker(): void {
  clearTimer();
  listeners.clear();
}

/** Re-renders the caller whenever a timed mute expires. */
export function useTimedMuteVersion(): number {
  return React.useSyncExternalStore(
    subscribeTimedMuteVersion,
    getTimedMuteVersion,
    getTimedMuteVersion,
  );
}
