// Phone layout ([data-phone], < 600px) bottom-tab state. Tabs map onto existing routes; the two list screens that have
// no route of their own (Channels list, Me) are an overlay flag shown over the current route.
import * as React from "react";

export type PhoneTab = "inbox" | "channels" | "search" | "term" | "me";
export type PhoneOverlay = "channels" | "search" | "me" | null;

let overlay: PhoneOverlay = null;
const listeners = new Set<() => void>();

export function setPhoneOverlay(next: PhoneOverlay): void {
  if (next === overlay) return;
  overlay = next;
  for (const l of listeners) l();
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
