/**
 * `navigator.mediaDevices` is absent in a non-secure context — a WKWebView that
 * did not get a secure origin exposes no `mediaDevices` at all. Touching it
 * unguarded from a mount-time effect throws before the user ever starts a
 * huddle, taking the whole React tree down with it.
 *
 * Returns the live `MediaDevices` object, or `null` when the API is missing.
 * Mirrors the guard already used in
 * `features/profile/lib/animatedAvatarCapture.ts`.
 */
export function availableMediaDevices(): MediaDevices | null {
  const media =
    typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
  return typeof media?.enumerateDevices === "function" ? media : null;
}

/** Raw error thrown when a huddle needs a mic but the API is unavailable. */
export const MICROPHONE_UNAVAILABLE_ERROR = "microphone_unavailable";
