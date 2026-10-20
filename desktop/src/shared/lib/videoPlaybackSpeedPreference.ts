import * as React from "react";

/**
 * Device-level video playback speed. Selecting a speed in any video player
 * persists it as the preference every later player starts from, so a viewer
 * who watches at 2x does not have to re-select it per video.
 */
export const VIDEO_PLAYBACK_SPEED_STORAGE_KEY = "buzz.media.videoPlaybackSpeed";

/** Selectable speeds, fastest first to match the control's menu order. */
export const VIDEO_PLAYBACK_SPEEDS = [
  2, 1.75, 1.5, 1.25, 1, 0.75, 0.5, 0.25,
] as const;

/** Playback speed used when no valid saved preference exists. */
export const DEFAULT_VIDEO_PLAYBACK_SPEED = 1;

const listeners = new Set<() => void>();
let videoPlaybackSpeed: number | null = null;
let listeningForStorageChanges = false;

/** True for speeds the control can actually represent. */
export function isVideoPlaybackSpeed(speed: number): boolean {
  return VIDEO_PLAYBACK_SPEEDS.some((option) => option === speed);
}

/** Parse a stored value, falling back when it is missing or unsupported. */
export function parseVideoPlaybackSpeed(
  value: string | null | undefined,
): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) && isVideoPlaybackSpeed(parsed)
    ? parsed
    : DEFAULT_VIDEO_PLAYBACK_SPEED;
}

function readStoredVideoPlaybackSpeed(): number {
  try {
    return parseVideoPlaybackSpeed(
      globalThis.localStorage?.getItem(VIDEO_PLAYBACK_SPEED_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_VIDEO_PLAYBACK_SPEED;
  }
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

function listenForStorageChanges(): void {
  if (listeningForStorageChanges || !globalThis.window?.addEventListener) {
    return;
  }
  globalThis.window.addEventListener("storage", (event) => {
    if (event.key === VIDEO_PLAYBACK_SPEED_STORAGE_KEY || event.key === null) {
      const nextSpeed = readStoredVideoPlaybackSpeed();
      if (nextSpeed === videoPlaybackSpeed) return;
      videoPlaybackSpeed = nextSpeed;
      notifyListeners();
    }
  });
  listeningForStorageChanges = true;
}

/**
 * Subscribe to playback-speed changes, including changes made in another
 * window. Returns an unsubscribe function.
 */
export function subscribeToVideoPlaybackSpeed(
  listener: () => void,
): () => void {
  listeners.add(listener);
  listenForStorageChanges();
  return () => {
    listeners.delete(listener);
  };
}

/** Return the current device-level playback-speed preference. */
export function getVideoPlaybackSpeed(): number {
  listenForStorageChanges();
  if (videoPlaybackSpeed === null) {
    videoPlaybackSpeed = readStoredVideoPlaybackSpeed();
  }
  return videoPlaybackSpeed;
}

/**
 * Persist the viewer's chosen speed and apply it to every mounted player.
 * Unsupported values are ignored so a stale caller cannot strand the control
 * on a speed it cannot display.
 */
export function setVideoPlaybackSpeed(speed: number): void {
  if (!isVideoPlaybackSpeed(speed)) return;
  const changed = getVideoPlaybackSpeed() !== speed;
  videoPlaybackSpeed = speed;
  try {
    globalThis.localStorage?.setItem(
      VIDEO_PLAYBACK_SPEED_STORAGE_KEY,
      String(speed),
    );
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  if (changed) notifyListeners();
}

/** Subscribe a player to the shared, persisted playback speed. */
export function useVideoPlaybackSpeed(): number {
  return React.useSyncExternalStore(
    subscribeToVideoPlaybackSpeed,
    getVideoPlaybackSpeed,
    () => DEFAULT_VIDEO_PLAYBACK_SPEED,
  );
}
