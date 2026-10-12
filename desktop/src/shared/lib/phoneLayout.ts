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
  installPhoneViewportHeight(mql);
  installPhoneTouchBehaviors(mql);
}

/** iOS: track the visual viewport (shrinks with the soft keyboard) as --phone-vh so the shell and composer stay on
 *  screen; Safari otherwise scrolls the layout viewport and pushes the composer under the keyboard. */
function installPhoneViewportHeight(mql: MediaQueryList): void {
  const vv = window.visualViewport;
  const root = document.documentElement;
  let maxH = 0;
  const sync = () => {
    if (!mql.matches) {
      root.style.removeProperty("--phone-vh");
      root.removeAttribute("data-phone-keyboard");
      return;
    }
    let h = vv ? vv.height : window.innerHeight;
    const standalone = isStandaloneDisplay();
    if (standalone) root.setAttribute("data-phone-standalone", "");
    else root.removeAttribute("data-phone-standalone");
    // The soft keyboard is up when the visual viewport is much shorter than the layout viewport (or than the
    // largest height seen, for engines that resize both).
    maxH = Math.max(maxH, window.innerHeight, h);
    const keyboard = h < maxH - 150;
    if (keyboard) root.setAttribute("data-phone-keyboard", "");
    else root.removeAttribute("data-phone-keyboard");
    // Installed iOS PWA: the visual/inner height can come up short of the full screen (status-bar and home
    // indicator insets subtracted), leaving a dead band under the tab bar. Without the keyboard, use the full height.
    if (standalone && !keyboard) h = Math.max(h, window.innerHeight, maxH);
    root.style.setProperty("--phone-vh", `${Math.round(h)}px`);
    if (vv && (window.scrollY !== 0 || vv.offsetTop !== 0)) window.scrollTo(0, 0);
  };
  sync();
  vv?.addEventListener("resize", sync);
  vv?.addEventListener("scroll", sync);
  window.addEventListener("resize", sync);
  mql.addEventListener("change", sync);
}

/** Installed web app (iOS "Add to Home Screen" or display-mode: standalone). */
export function isStandaloneDisplay(): boolean {
  if (typeof window === "undefined") return false;
  const nav = navigator as Navigator & { standalone?: boolean };
  return nav.standalone === true || (typeof window.matchMedia === "function" && window.matchMedia("(display-mode: standalone)").matches);
}

const ROW_SELECTOR = '[data-testid="message-row"]';
const INTERACTIVE_SELECTOR =
  'a,button,input,textarea,select,label,[contenteditable="true"],[role="button"],[role="menu"],[role="menuitem"],[data-message-action-rail],[data-radix-popper-content-wrapper]';
const OPEN_ATTR = "data-phone-actions-open";

function isEditable(el: Element | null): el is HTMLElement {
  return !!el && (el instanceof HTMLElement) && (el.isContentEditable || /^(INPUT|TEXTAREA)$/.test(el.tagName));
}

/** Phone touch: a tap or scroll start in the message list dismisses the keyboard; message actions (hidden on phone)
 *  open for one row on tap or long-press and close on a tap elsewhere. */
function installPhoneTouchBehaviors(mql: MediaQueryList): void {
  let pressTimer: number | undefined;
  let pressStart: { x: number; y: number } | null = null;
  let suppressClick = false;
  const closeAll = (except?: Element | null) => {
    for (const el of document.querySelectorAll(`[${OPEN_ATTR}]`)) if (el !== except) el.removeAttribute(OPEN_ATTR);
  };
  const openRow = (row: Element) => {
    closeAll(row);
    row.setAttribute(OPEN_ATTR, "");
  };
  document.addEventListener(
    "touchstart",
    (event) => {
      if (!mql.matches) return;
      const target = event.target as Element | null;
      if (!target?.closest?.('[data-testid="message-timeline"]')) return;
      if (target.closest('[data-testid="message-composer"],[data-radix-popper-content-wrapper]')) return;
      const active = document.activeElement;
      if (isEditable(active) && !active.contains(target)) active.blur();
    },
    { capture: true, passive: true },
  );
  document.addEventListener(
    "pointerdown",
    (event) => {
      if (!mql.matches || event.pointerType === "mouse") return;
      const target = event.target as Element | null;
      const row = target?.closest?.(ROW_SELECTOR);
      window.clearTimeout(pressTimer);
      if (!row || target?.closest(INTERACTIVE_SELECTOR)) return;
      pressStart = { x: event.clientX, y: event.clientY };
      pressTimer = window.setTimeout(() => {
        openRow(row);
        suppressClick = true;
        navigator.vibrate?.(10);
      }, 450);
    },
    true,
  );
  const cancelPress = (event: PointerEvent) => {
    if (pressStart && Math.hypot(event.clientX - pressStart.x, event.clientY - pressStart.y) < 10 && event.type === "pointermove") return;
    window.clearTimeout(pressTimer);
    pressStart = null;
  };
  document.addEventListener("pointermove", cancelPress, true);
  document.addEventListener("pointercancel", cancelPress, true);
  document.addEventListener("pointerup", () => window.clearTimeout(pressTimer), true);
  document.addEventListener(
    "click",
    (event) => {
      if (!mql.matches) return;
      if (suppressClick) {
        suppressClick = false;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      const target = event.target as Element | null;
      // Some timeline click handlers re-focus the composer; a tap in the list keeps the keyboard down.
      if (
        target?.closest?.('[data-testid="message-timeline"]') &&
        !target.closest('[data-testid="message-composer"],[data-radix-popper-content-wrapper]')
      ) {
        window.setTimeout(() => {
          const active = document.activeElement;
          if (isEditable(active) && !active.closest('[data-radix-popper-content-wrapper]')) active.blur();
        }, 0);
      }
      if (target?.closest?.("[data-message-action-rail],[data-radix-popper-content-wrapper]")) return;
      const row = target?.closest?.(ROW_SELECTOR);
      if (row && !target?.closest(INTERACTIVE_SELECTOR)) {
        // A tap on the message body shows its actions (the "N replies" link still opens the thread).
        event.preventDefault();
        event.stopPropagation();
        if (row.hasAttribute(OPEN_ATTR)) row.removeAttribute(OPEN_ATTR);
        else openRow(row);
        return;
      }
      closeAll();
    },
    true,
  );
  document.addEventListener(
    "contextmenu",
    (event) => {
      if (!mql.matches) return;
      const target = event.target as Element | null;
      const row = target?.closest?.(ROW_SELECTOR);
      if (row && !target?.closest("a,img,video")) {
        event.preventDefault();
        openRow(row);
      }
    },
    true,
  );
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
