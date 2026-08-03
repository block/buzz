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
/**
 * Latched so the diagnostic below is emitted at most once per process.
 *
 * This is environment state, not community state: whether the webview exposes
 * `mediaDevices` cannot change when the user switches communities. It is
 * therefore deliberately NOT wired into `resetCommunityState()`.
 */
let missingApiWarned = false;

export function availableMediaDevices(): MediaDevices | null {
  const media =
    typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
  if (typeof media?.enumerateDevices === "function") {
    return media;
  }

  // One line, once. Callers hit this from three sites and mount-time effects
  // run twice under `React.StrictMode`, so an unguarded log would emit five
  // identical lines and bury the signal it exists to provide.
  if (!missingApiWarned) {
    missingApiWarned = true;
    console.warn(
      "[mediaDevices] navigator.mediaDevices is unavailable (non-secure context); huddle audio is disabled in this window",
    );
  }
  return null;
}

/** Raw error thrown when a huddle needs a mic but the API is unavailable. */
export const MICROPHONE_UNAVAILABLE_ERROR = "microphone_unavailable";
