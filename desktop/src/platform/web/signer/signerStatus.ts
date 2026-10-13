import { ensureStatusStyles } from "@/shared/ui/statusPills";
import { bottomClearancePx, getSignerToastPrefs, signerToastDelayMs, subscribeSignerToastPrefs } from "./signerToastPrefs";
import { ESCALATE_WAIT_MS, nip07EscalateWaitMs, signerHealth } from "./signerHealth";
// Tiny vanilla-DOM "Waiting for your signer…" indicator, independent of the React app tree.
type St = { pending: number; authUrl: string | null; queued: number; labels: string[] };
let st: St = { pending: 0, authUrl: null, queued: 0, labels: [] };
const listeners = new Set<() => void>();
let el: HTMLDivElement | null = null;
let showTimer: ReturnType<typeof setTimeout> | undefined;
/** The user hid the toast: stay hidden until nothing is pending (the next wait shows it again). */
let dismissed = false;

function render() {
  if (st.pending === 0 && st.queued === 0) {
    dismissed = false;
    clearTimeout(showTimer);
    showTimer = undefined;
    el?.remove();
    el = null;
    return;
  }
  if (dismissed) return;
  const prefs = getSignerToastPrefs();
  const delay = signerToastDelayMs(prefs.mode, !!st.authUrl);
  if (delay === null) {
    // Off: hide (an auth URL that blocks the action still shows; see signerToastDelayMs).
    clearTimeout(showTimer);
    showTimer = undefined;
    el?.remove();
    el = null;
    return;
  }
  if (el) { update(); place(); return; }
  if (showTimer) {
    // Waiting on a "problems" delay and an auth URL just arrived: show now.
    if (delay > 0) return;
    clearTimeout(showTimer);
  }
  showTimer = setTimeout(() => {
    showTimer = undefined;
    if (st.pending === 0 && st.queued === 0) return;
    el = document.createElement("div");
    el.setAttribute("data-testid", "signer-waiting");
    el.setAttribute("role", "status");
    document.documentElement.appendChild(el);
    update();
    place();
  }, delay);
}
const DOCKED = '[data-testid="phone-tabbar"], [data-testid="message-composer"], .buzz-terminal-touch-key';
function cssPx(name: string): number {
  const v = parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name));
  return Number.isFinite(v) ? v : 0;
}
/**
 * True overlay: position:fixed on <html> (outside #root / body flow), placed by `top` in visual-viewport terms so
 * iOS keyboard pans can't drag it into or over content. Only the toast itself takes pointer events.
 * Top: under the safe area. Bottom: above the tab bar / composer / terminal keys.
 */
function place() {
  if (!el) return;
  const phone = window.matchMedia?.("(max-width: 599px)").matches;
  const { position } = getSignerToastPrefs();
  el.setAttribute("data-position", position);
  // Same look as the shared status pills (statusPills.ts): solid themed card, border, 14px radius.
  const look =
    "max-width:calc(100vw - 16px);background:hsl(var(--card));color:hsl(var(--card-foreground));border:1px solid hsl(var(--border));" +
    `border-radius:14px;padding:8px 12px;font:${phone ? 13 : 14}px/1.35 Inter,system-ui,sans-serif;box-shadow:0 6px 24px rgba(0,0,0,.16);gap:10px`;
  const fixedCss = "position:fixed;left:50%;right:auto;bottom:auto;margin:0;transform:translateX(-50%);z-index:2147483647;display:flex;align-items:center;pointer-events:auto;box-sizing:border-box;";
  // Style first (off-screen) so its real height is known before placing it.
  el.style.cssText = `${fixedCss}top:-9999px;${look}`;
  const vv = window.visualViewport;
  const vTop = vv?.offsetTop ?? 0;
  const vh = vv?.height ?? window.innerHeight;
  let top: number;
  if (position === "top") {
    const rootEl = document.getElementById("root");
    const safeTop = phone && rootEl ? parseFloat(getComputedStyle(rootEl).paddingTop) || 0 : 0; // = --phone-safe-top, resolved
    // Under the nav row / tab pill like every other popup, and under any pills already in the shared host.
    top = vTop + safeTop + 48;
    const hostBox = document.querySelector(".buzz-status-host")?.getBoundingClientRect();
    if (hostBox && hostBox.height > 0) top = Math.max(top, hostBox.bottom + 8);
  } else {
    const tops = [...document.querySelectorAll(DOCKED)]
      .map((n) => n.getBoundingClientRect())
      .filter((r) => r.width > 0 && r.height > 0)
      .map((r) => r.top - vTop);
    const clear = bottomClearancePx(vh, tops, Math.max(cssPx("--phone-safe-bottom"), 0));
    const h = el.offsetHeight || 40;
    top = vTop + vh - clear - h;
  }
  el.style.cssText = `${fixedCss}top:${Math.round(top)}px;${look}`;
}
if (typeof window !== "undefined") {
  const re = () => place();
  window.addEventListener("resize", re);
  window.visualViewport?.addEventListener("resize", re);
  window.visualViewport?.addEventListener("scroll", re);
  window.addEventListener("hashchange", () => setTimeout(re, 300));
  subscribeSignerToastPrefs(() => {
    if (el && getSignerToastPrefs().mode === "off" && !st.authUrl) { el.remove(); el = null; }
    place();
    render();
  });
}
function update() {
  if (!el) return;
  el.textContent = "";
  ensureStatusStyles();
  const spin = document.createElement("span");
  spin.className = "buzz-status-spin";
  spin.style.flexShrink = "0";
  el.appendChild(spin);
  const t = document.createElement("span");
  const n = Math.max(st.pending, st.queued);
  const what = st.labels.slice(0, 2).join(", ");
  t.textContent = `Waiting for your signer…${what ? ` (${what}${st.labels.length > 2 ? ", …" : ""})` : ""}${n > 1 ? ` · ${n} pending` : ""}`;
  el.appendChild(t);
  if (st.authUrl) {
    const a = document.createElement("a");
    a.href = st.authUrl;
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = "Open signer to approve";
    a.setAttribute("data-testid", "signer-auth-url");
    a.style.cssText = "color:hsl(var(--primary));font-weight:600;text-decoration:underline";
    el.appendChild(a);
  }
  const x = document.createElement("button");
  x.type = "button";
  x.textContent = "Hide";
  x.setAttribute("data-testid", "signer-waiting-hide");
  x.style.cssText = "background:none;border:0;color:hsl(var(--muted-foreground));font:inherit;cursor:pointer;padding:0";
  x.onclick = () => {
    dismissed = true;
    el?.remove();
    el = null;
  };
  el.appendChild(x);
}
let escalateTimer: ReturnType<typeof setTimeout> | undefined;
let waitStart = 0;
// "Waiting for your signer…" that lasts too long (and isn't the user approving in the signer app) becomes the unresponsive state.
// NIP-07: the extension may be waiting for a popup approval and answers in a queue, so only count time with NO answer at all,
// and use a much longer limit. Every completed request (signerHealth.success) pushes the deadline out.
function watchEscalation() {
  const waiting = st.pending > 0 || st.queued > 0;
  if (!waiting || st.authUrl) {
    clearTimeout(escalateTimer);
    escalateTimer = undefined;
    return;
  }
  if (escalateTimer) return;
  waitStart = Date.now();
  const arm = (ms: number) => {
    escalateTimer = setTimeout(() => {
      escalateTimer = undefined;
      if (!((st.pending > 0 || st.queued > 0) && !st.authUrl)) return;
      if (signerHealth.signerKind() === "nip07") {
        const quietFor = Date.now() - Math.max(signerHealth.lastAlive(), waitStart);
        if (quietFor < nip07EscalateWaitMs()) return arm(nip07EscalateWaitMs() - quietFor);
      }
      signerHealth.escalate("long-wait");
    }, ms);
  };
  arm(signerHealth.signerKind() === "nip07" ? nip07EscalateWaitMs() : ESCALATE_WAIT_MS);
}
export const signerStatus = {
  get: () => st,
  set(p: Partial<St>) {
    st = { ...st, ...p };
    render();
    watchEscalation();
    if (el) update();
    listeners.forEach((f) => f());
  },
  subscribe(f: () => void) {
    listeners.add(f);
    return () => void listeners.delete(f);
  },
};
