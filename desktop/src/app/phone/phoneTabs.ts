// Phone layout ([data-phone], < 600px) bottom-tab state. Tabs map onto existing routes; the two list screens that have
// no route of their own (Channels list, Me) are an overlay flag shown over the current route.
import * as React from "react";

export type PhoneTab = "inbox" | "channels" | "search" | "term" | "me";
export type PhoneOverlay = "channels" | "search" | "me" | null;

let overlay: PhoneOverlay = null;
const listeners = new Set<() => void>();

function setOverlayValue(next: PhoneOverlay): void {
  if (next === overlay) return;
  overlay = next;
  for (const l of listeners) l();
}

/**
 * Phone: the list screens (Channels, Me, Search) live in browser history (as `phoneOverlay` in the entry's state), so
 * the shared Back / Forward controls step through them like any other screen. Settings opened from Me goes back to Me,
 * never to the channel underneath. AppShell registers the navigator while the phone layout is active; elsewhere this
 * stays a plain in-memory flag.
 */
type OverlayNavigator = (next: PhoneOverlay, opts?: { replace?: boolean }) => void;
let navigator: OverlayNavigator | null = null;
export function registerPhoneOverlayNavigator(fn: OverlayNavigator): () => void {
  navigator = fn;
  return () => {
    if (navigator === fn) navigator = null;
  };
}
export function setPhoneOverlay(next: PhoneOverlay, opts?: { replace?: boolean }): void {
  if (navigator) navigator(next, opts);
  else setOverlayValue(next);
}
/** AppShell mirrors the current history entry's overlay into the store. */
export function syncPhoneOverlayFromHistory(next: PhoneOverlay): void {
  setOverlayValue(next);
}
/**
 * The overlay of a history entry. Router navigations copy the current entry's state into the next one, so the overlay is
 * stored with the href it was opened on and only counts on that same href (a channel opened from the list is not "the
 * list" just because it inherited the state).
 */
export function overlayFromHistoryState(state: unknown, href: string): PhoneOverlay {
  const st = state as { phoneOverlay?: unknown; phoneOverlayHref?: unknown } | null | undefined;
  if (st?.phoneOverlayHref !== href) return null;
  const v = st?.phoneOverlay;
  return v === "channels" || v === "me" || v === "search" ? v : null;
}

export function usePhoneOverlay(): PhoneOverlay {
  return React.useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => overlay,
    () => null,
  );
}

const ME_PATHS = ["/agents", "/settings", "/bots", "/browsers", "/projects", "/workflows", "/pulse", "/reminders", "/pins"];

/** Which tab is active for a pathname (deep links land on the matching tab). */
export function phoneTabForPath(pathname: string, current: PhoneOverlay): PhoneTab {
  if (current === "channels") return "channels";
  if (current === "me") return "me";
  if (current === "search") return "search";
  if (pathname.startsWith("/channels/") || pathname === "/messages/new") return "channels";
  if (pathname.startsWith("/buzz-term")) return "term";
  if (ME_PATHS.some((p) => pathname === p || pathname.startsWith(`${p}/`))) return "me";
  return "inbox";
}

/** Full-screen conversation views hide the tab bar so the composer has room (iOS chat pattern). */
export function phoneHidesTabBar(pathname: string, current: PhoneOverlay): boolean {
  if (current) return false;
  return pathname.startsWith("/channels/") || pathname === "/messages/new" || pathname === "/settings";
}
