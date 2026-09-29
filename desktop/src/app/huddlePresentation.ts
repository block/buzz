/** Main-app huddle chrome: companion OS window only (drawer type retained for compat). */
export type HuddlePresentation = "none" | "drawer" | "window";

/**
 * Main webview must NEVER mount the drawer / HuddleBar.
 *
 * Product: huddle UI lives only on the companion (or dedicated huddle surface).
 * Mounting on main caused a dock flash on start/join and briefly connected a
 * second LiveKit share session. `"drawer"` remains in the presentation enum for
 * legacy callers but is not a mount path.
 */
export function shouldMountMainHuddleDrawerBar(
  _presentation: HuddlePresentation,
): boolean {
  return false;
}

/**
 * AppHuddleShell dock chrome (mount HuddleBar + lift app surface).
 *
 * Window identity is the gate — not main `presentation`:
 * - Companion room webview (`isRoom`, label `huddle-<uuid>`) ALWAYS shows the
 *   control dock. Main presentation stays `"none"`/`"window"` and must not be
 *   required for companion CSS lift (`buzz-huddle-app-surface-room-open`).
 * - Main webview NEVER shows the dock (`shouldMountMainHuddleDrawerBar`).
 *
 * Regression: quality pass stopped promoting presentation→`"drawer"` on
 * visibility, so companion kept `isDrawerOpen=false`, left the app surface at
 * `bottom:0` / z-10, and covered the mounted bar in the z-2 drawer slot.
 */
export function shouldShowHuddleDockChrome(options: {
  isRoom: boolean;
  presentation: HuddlePresentation;
}): boolean {
  return (
    options.isRoom || shouldMountMainHuddleDrawerBar(options.presentation)
  );
}

type ReconcileOptions = {
  /** True while `open_huddle_window` has not settled. */
  openInFlight?: boolean;
  /** True when an ephemeral huddle session is still active. */
  huddleActive?: boolean;
};

/**
 * Reconcile React presentation with native `huddle_companion_window_exists`.
 * Native existence wins: if the companion webview is in the window map, main
 * must be `"window"` so drawer chrome stays unmounted (XOR). Returns null when
 * the current presentation already matches.
 *
 * When the companion is gone, demote to `"none"` — never `"drawer"` on main.
 */
export function reconcilePresentationWithNativeExists(
  presentation: HuddlePresentation,
  companionExists: boolean,
  options: ReconcileOptions = {},
): HuddlePresentation | null {
  if (companionExists) {
    return presentation === "window" ? null : "window";
  }
  if (options.openInFlight) {
    return null;
  }
  if (presentation !== "window" && presentation !== "drawer") {
    return null;
  }
  // Companion gone (or legacy drawer): keep main chrome unmounted.
  return "none";
}

/**
 * Main must never promote to drawer. Kept as a named gate so call sites stay
 * explicit; always false under the companion-only product rule.
 */
export function shouldPromoteNoneToDrawer(_options: {
  presentation: HuddlePresentation;
  companionExists: boolean;
  openInFlight: boolean;
  startPending: boolean;
}): boolean {
  return false;
}

/**
 * Coalesce `open_huddle_window` only while an invoke is in flight for the same
 * channel. A settled prior open must not skip native open — the OS companion
 * may already be gone, and flipping presentation to "window" without a real
 * window leaves the session with no huddle surface.
 */
export function shouldCoalesceHuddleCompanionOpen(options: {
  sameChannel: boolean;
  openInFlight: boolean;
}): boolean {
  return options.sameChannel && options.openInFlight;
}
