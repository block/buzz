// Phone-only layout switch: viewport narrower than 600px (iPhone-class). iPad (744px+) and desktop never match.
// Sets `data-phone` on <html> so phone styles can be scoped with [data-phone]; usePhoneLayout() for components.
import * as React from "react";

export const PHONE_MAX_WIDTH_PX = 599;
const query = () => `(max-width: ${PHONE_MAX_WIDTH_PX}px)`;

export function isPhoneLayout(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia(query()).matches;
}

/** Keeps `data-phone` on the root element in sync with the viewport. Call once at startup. */
export function installPhoneLayoutAttribute(): void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  const mql = window.matchMedia(query());
  const sync = () => {
    if (mql.matches) document.documentElement.setAttribute("data-phone", "");
    else document.documentElement.removeAttribute("data-phone");
  };
  sync();
  mql.addEventListener("change", sync);
}

export function usePhoneLayout(): boolean {
  return React.useSyncExternalStore(
    (cb) => {
      if (typeof window === "undefined" || typeof window.matchMedia !== "function") return () => {};
      const mql = window.matchMedia(query());
      mql.addEventListener("change", cb);
      return () => mql.removeEventListener("change", cb);
    },
    isPhoneLayout,
    () => false,
  );
}
