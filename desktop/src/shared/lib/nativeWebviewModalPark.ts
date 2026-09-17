import {
  hideAllPlaygroundWebviews,
  PLAYGROUND_WEBVIEW_RESTORE_EVENT,
} from "@/features/playground/lib/webview";
import {
  hideAllPinWebviews,
  PIN_WEBVIEW_RESTORE_EVENT,
} from "@/features/pinned-sites/lib/pinWebview";

/** Re-export for modal hosts / tests. */
export { PLAYGROUND_WEBVIEW_RESTORE_EVENT };

/**
 * Ref-count of blocking Buzz overlays (Dialog / Sheet / AlertDialog content,
 * and any host that opts in via {@link acquireNativeWebviewModalPark}).
 *
 * Native pin/playground WKWebViews paint above the React layer, so while any
 * blocking overlay is open we park those main-window children. When the last
 * overlay closes we restore surfaces that are still mounted (active pin /
 * open link panel / playground stage).
 */
let parkDepth = 0;

export function getNativeWebviewModalParkDepth(): number {
  return parkDepth;
}

export function isNativeWebviewModalParked(): boolean {
  return parkDepth > 0;
}

function dispatchWindowEvent(name: string): void {
  if (typeof window === "undefined") return;
  if (typeof window.dispatchEvent !== "function") return;
  const EventCtor = window.Event;
  if (typeof EventCtor !== "function") return;
  try {
    window.dispatchEvent(new EventCtor(name));
  } catch {
    // Node test hosts may lack a DOM Event implementation.
  }
}

/**
 * Park every main-window native webview that can cover React modals.
 * Safe to call when none are showing. Bumps pin hide-epoch so in-flight
 * shows cannot resurrect orphans (same class as PRs #53/#54/#59).
 */
export function parkNativeWebviewsForModal(): void {
  void hideAllPinWebviews();
  void hideAllPlaygroundWebviews();
}

/**
 * Begin a blocking-overlay park. First acquirer hides native children;
 * nested dialogs only bump the depth.
 */
export function acquireNativeWebviewModalPark(): void {
  parkDepth += 1;
  if (parkDepth === 1) {
    parkNativeWebviewsForModal();
  }
}

/**
 * End a blocking-overlay park. Last releaser restores mounted surfaces
 * unless another overlay still holds the park.
 */
export function releaseNativeWebviewModalPark(): void {
  if (parkDepth <= 0) {
    parkDepth = 0;
    return;
  }
  parkDepth -= 1;
  if (parkDepth === 0) {
    notifyNativeWebviewRestore();
  }
}

/**
 * Ask mounted pin / link / playground hosts to re-show after a modal park.
 * No-ops while a modal park is still held so playground dismiss cannot
 * restore a pin underneath an open dialog.
 */
export function notifyNativeWebviewRestore(): void {
  if (parkDepth > 0) return;
  dispatchWindowEvent(PIN_WEBVIEW_RESTORE_EVENT);
  dispatchWindowEvent(PLAYGROUND_WEBVIEW_RESTORE_EVENT);
}

/**
 * Restore pins/link panel only (playground dismiss / navigate-away).
 * Skips while a blocking modal is open.
 */
export function notifyPinWebviewRestore(): void {
  if (parkDepth > 0) return;
  dispatchWindowEvent(PIN_WEBVIEW_RESTORE_EVENT);
}

/** Test helper — clear depth without restoring. */
export function resetNativeWebviewModalPark(): void {
  parkDepth = 0;
}
