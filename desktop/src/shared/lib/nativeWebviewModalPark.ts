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
  parkWithStandIns(false);
}

/**
 * Before-park hooks let a mounted native surface put a DOM snapshot of its
 * page at the same bounds, so the site appears to stay put under a menu or
 * dialog instead of vanishing. A hook returns void when it could show a
 * cached snapshot immediately, or a Promise when it must capture first (the
 * park then waits for it, at most {@link STAND_IN_CAPTURE_WAIT_MS}).
 */
type BeforeParkHook = () => void | Promise<unknown>;
const beforeParkHooks = new Set<BeforeParkHook>();
export const STAND_IN_CAPTURE_WAIT_MS = 300;

export function registerBeforeParkHook(hook: BeforeParkHook): () => void {
  beforeParkHooks.add(hook);
  return () => {
    beforeParkHooks.delete(hook);
  };
}

let parkSerial = 0;

function hideNativeNow(): void {
  void hideAllPinWebviews();
  void hideAllPlaygroundWebviews();
}

function parkWithStandIns(fromAcquire: boolean): void {
  const serial = ++parkSerial;
  const pending: Promise<unknown>[] = [];
  for (const hook of beforeParkHooks) {
    try {
      const result = hook();
      if (result && typeof (result as Promise<unknown>).then === "function") {
        pending.push(result as Promise<unknown>);
      }
    } catch {
      // A failed stand-in must never block the park.
    }
  }
  if (pending.length === 0) {
    hideNativeNow();
    return;
  }
  void Promise.race([
    Promise.allSettled(pending),
    new Promise((resolve) => setTimeout(resolve, STAND_IN_CAPTURE_WAIT_MS)),
  ]).then(() => {
    // Released (or re-parked) while capturing: do not hide behind a closed menu.
    if (fromAcquire && parkDepth === 0) return;
    if (serial !== parkSerial && fromAcquire) return;
    hideNativeNow();
  });
}

const parkListeners = new Set<(parked: boolean) => void>();

/** Notified when the modal park starts (true) and ends (false). */
export function subscribeNativeWebviewModalPark(
  listener: (parked: boolean) => void,
): () => void {
  parkListeners.add(listener);
  return () => {
    parkListeners.delete(listener);
  };
}

function notifyParkListeners(parked: boolean): void {
  for (const listener of parkListeners) {
    try {
      listener(parked);
    } catch {
      // ignore
    }
  }
}

/**
 * Begin a blocking-overlay park. First acquirer hides native children;
 * nested dialogs only bump the depth.
 */
export function acquireNativeWebviewModalPark(): void {
  parkDepth += 1;
  if (parkDepth === 1) {
    parkWithStandIns(true);
    notifyParkListeners(true);
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
    notifyParkListeners(false);
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
