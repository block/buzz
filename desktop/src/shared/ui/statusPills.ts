// One host for every transient popup on web: status pills ("Reconnecting to relay…", "Connected", Direct reconnecting,
// "Waiting for your signer…"), the update prompt, the notifications banner and calm error notes. Framework-free (vanilla
// DOM) so the non-React web modules (updateCheck, push banner, signer status) and React hooks share it.
//
// ── Look ───────────────────────────────────────────────────────────────────────────────────────────────────────────
//   Solid themed card: background hsl(--card), text hsl(--card-foreground), 1px hsl(--border), radius 14px, soft shadow.
//   In-progress states show a spinner; success a check; info/error a muted dot. No red text anywhere: errors read in the
//   normal foreground color with a calm description. Actions are outlined primary buttons.
//
// ── Placement / z-order ────────────────────────────────────────────────────────────────────────────────────────────
//   One fixed, centered, stacked column. Top = safe area + 48px, i.e. just under the 40px phone nav row and the floating
//   tab-count pill (and under desktop/iPad top chrome), so a popup never covers Back/Forward/Home or the pill.
//   z-index STATUS_Z (2147482000): above app content, the nav row (125) and the tab pill (130); below the full-screen
//   signer / permission prompts (2147483646+) that must stay on top.
//
// ── Timing policy (STATUS_TIMING) ──────────────────────────────────────────────────────────────────────────────────
//   progress  ("Reconnecting…"): shown only after GRACE_MS (1.5s) of the state lasting, so brief blips show nothing;
//             persists while the state lasts; never needs a click to recover.
//   success   ("Connected"/"Back online"): shown for SUCCESS_MS (2s) and ONLY if its progress pill was actually
//             visible. A blip that never showed "Reconnecting" never shows "Connected" either.
//   info/error (one-shot notes, sonner toasts): NOTE_MS (4s), then fade.
//   action    (update prompt, "Turn on notifications"): persists until acted on, or until the condition clears
//             (e.g. permission granted). An optional Hide may exist but is never required.
//   Every exit fades over FADE_MS (200ms).
// Inline cards (ReconnectCard inside Buzz Term / My Agents) follow the same GRACE_MS / SUCCESS_MS.

export const STATUS_TIMING = {
  GRACE_MS: 1_500,
  SUCCESS_MS: 2_000,
  NOTE_MS: 4_000,
  FADE_MS: 200,
} as const;
export const STATUS_Z = 2_147_482_000;

export type StatusTone = "progress" | "success" | "info" | "action";
export type StatusAction = { label: string; onClick: () => void; testId?: string };
export type StatusSpec = {
  tone: StatusTone;
  title: string;
  description?: string;
  action?: StatusAction;
  /** Optional, never required (the state recovers on its own). */
  onHide?: () => void;
  testId?: string;
};

const STYLE_ID = "buzz-status-pill-style";
const CSS = `
.buzz-status-host{position:fixed;left:50%;transform:translateX(-50%);top:calc(var(--phone-safe-top, env(safe-area-inset-top, 0px)) + 48px);
  z-index:${STATUS_Z};display:flex;flex-direction:column;align-items:center;gap:8px;width:min(calc(100vw - 16px),520px);pointer-events:none}
.buzz-status-pill{pointer-events:auto;box-sizing:border-box;display:flex;align-items:center;gap:10px;max-width:100%;padding:8px 12px;
  border-radius:14px;background:hsl(var(--card));color:hsl(var(--card-foreground));border:1px solid hsl(var(--border));
  box-shadow:0 6px 24px rgba(0,0,0,.16);font:14px/1.35 Inter,system-ui,-apple-system,sans-serif;opacity:1;
  transition:opacity ${STATUS_TIMING.FADE_MS}ms ease,transform ${STATUS_TIMING.FADE_MS}ms ease;animation:buzz-status-in ${STATUS_TIMING.FADE_MS}ms ease-out}
.buzz-status-pill[data-leaving]{opacity:0;transform:translateY(-4px)}
.buzz-status-pill[data-wide]{width:100%}
.buzz-status-icon{flex-shrink:0;width:26px;height:26px;border-radius:9999px;display:grid;place-items:center;background:hsl(var(--muted));color:hsl(var(--muted-foreground))}
.buzz-status-spin{width:14px;height:14px;border-radius:9999px;border:2px solid currentColor;border-right-color:transparent;animation:buzz-status-rot .8s linear infinite}
.buzz-status-dot{width:8px;height:8px;border-radius:9999px;background:hsl(var(--primary))}
.buzz-status-text{min-width:0;flex:1;display:flex;flex-direction:column}
.buzz-status-title{font-weight:600}
.buzz-status-desc{font-size:12px;color:hsl(var(--muted-foreground))}
.buzz-status-btn{flex-shrink:0;border:1px solid hsl(var(--primary));border-radius:10px;padding:5px 12px;font:600 13px/1.2 inherit;font-family:inherit;background:transparent;color:hsl(var(--primary));cursor:pointer}
.buzz-status-btn:disabled{opacity:.6}
.buzz-status-hide{flex-shrink:0;border:0;background:transparent;color:hsl(var(--muted-foreground));font:inherit;font-size:13px;padding:2px 4px;cursor:pointer}
@keyframes buzz-status-rot{to{transform:rotate(360deg)}}
@keyframes buzz-status-in{from{opacity:0;transform:translateY(-4px)}to{opacity:1;transform:none}}
`;

let host: HTMLDivElement | null = null;
const nodes = new Map<string, HTMLDivElement>();
// Display order: progress/success first, then actions (update prompt, banner), then notes.
const ORDER: Record<StatusTone, number> = { progress: 0, success: 0, action: 1, info: 2 };

/** Inject the shared popup styles (also used by the signer status toast's spinner). */
export function ensureStatusStyles(): void {
  if (typeof document === "undefined") return;
  if (!document.getElementById(STYLE_ID)) {
    const st = document.createElement("style");
    st.id = STYLE_ID;
    st.textContent = CSS;
    document.head.appendChild(st);
  }
}

function ensureHost(): HTMLDivElement | null {
  if (typeof document === "undefined") return null;
  ensureStatusStyles();
  if (host?.isConnected) return host;
  host = document.createElement("div");
  host.className = "buzz-status-host";
  host.setAttribute("data-testid", "status-host");
  document.body.appendChild(host);
  return host;
}

const CHECK_SVG =
  '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"/></svg>';

function build(id: string, spec: StatusSpec): HTMLDivElement {
  const el = document.createElement("div");
  el.className = "buzz-status-pill";
  el.setAttribute("role", "status");
  el.setAttribute("aria-live", "polite");
  el.setAttribute("data-status-id", id);
  el.setAttribute("data-tone", spec.tone);
  el.setAttribute("data-testid", spec.testId ?? `status-${id}`);
  if (spec.description || spec.action) el.setAttribute("data-wide", "");
  const icon = document.createElement("span");
  icon.className = "buzz-status-icon";
  if (spec.tone === "progress") {
    const s = document.createElement("span");
    s.className = "buzz-status-spin";
    icon.append(s);
  } else if (spec.tone === "success") icon.innerHTML = CHECK_SVG;
  else {
    const d = document.createElement("span");
    d.className = "buzz-status-dot";
    icon.append(d);
  }
  const text = document.createElement("span");
  text.className = "buzz-status-text";
  const t = document.createElement("span");
  t.className = "buzz-status-title";
  t.textContent = spec.title;
  text.append(t);
  if (spec.description) {
    const d = document.createElement("span");
    d.className = "buzz-status-desc";
    d.textContent = spec.description;
    text.append(d);
  }
  el.append(icon, text);
  if (spec.action) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "buzz-status-btn";
    b.textContent = spec.action.label;
    if (spec.action.testId) b.setAttribute("data-testid", spec.action.testId);
    const run = spec.action.onClick;
    b.onclick = () => run();
    el.append(b);
  }
  if (spec.onHide) {
    const h = document.createElement("button");
    h.type = "button";
    h.className = "buzz-status-hide";
    h.textContent = "Hide";
    h.setAttribute("aria-label", "Hide");
    const run = spec.onHide;
    h.onclick = () => run();
    el.append(h);
  }
  (el as HTMLDivElement & { _order?: number })._order = ORDER[spec.tone];
  return el;
}

/** Show (or replace) one popup by id. */
export function showStatus(id: string, spec: StatusSpec): HTMLDivElement | null {
  const h = ensureHost();
  if (!h) return null;
  const prev = nodes.get(id);
  const el = build(id, spec);
  if (prev && !prev.hasAttribute("data-leaving")) {
    el.style.animation = "none"; // in-place update, no re-entrance
    prev.replaceWith(el);
  } else {
    prev?.remove();
    const order = ORDER[spec.tone];
    const after = [...h.children].find((c) => ((c as HTMLDivElement & { _order?: number })._order ?? 0) > order);
    h.insertBefore(el, after ?? null);
  }
  nodes.set(id, el);
  return el;
}

/** Fade out and remove one popup. */
export function clearStatus(id: string): void {
  const el = nodes.get(id);
  if (!el) return;
  nodes.delete(id);
  el.setAttribute("data-leaving", "");
  window.setTimeout(() => el.remove(), STATUS_TIMING.FADE_MS);
}

export function hasStatus(id: string): boolean {
  return nodes.has(id);
}

/** One-shot note (info / calm error): NOTE_MS then fade. */
export function noteStatus(id: string, title: string, description?: string): void {
  showStatus(id, { tone: "info", title, description });
  const el = nodes.get(id);
  window.setTimeout(() => {
    if (nodes.get(id) === el) clearStatus(id);
  }, STATUS_TIMING.NOTE_MS);
}

type Tracked = { grace?: number; success?: number; visible: boolean; active: boolean };
const tracked = new Map<string, Tracked>();

/**
 * Drive a progress → success pill from a boolean, applying the timing policy: show after GRACE_MS, persist while active,
 * then SUCCESS_MS of `successTitle` only if the progress pill was visible. Call it on every state change (idempotent).
 */
export function trackProgress(
  id: string,
  active: boolean,
  copy: { title: string; description?: string; successTitle?: string; testId?: string },
): void {
  const t = tracked.get(id) ?? { visible: false, active: false };
  tracked.set(id, t);
  if (active) {
    if (t.success) {
      window.clearTimeout(t.success);
      t.success = undefined;
    }
    t.active = true;
    if (t.visible) {
      showStatus(id, { tone: "progress", title: copy.title, description: copy.description, testId: copy.testId });
      return;
    }
    if (!t.grace) {
      t.grace = window.setTimeout(() => {
        t.grace = undefined;
        if (!t.active) return;
        t.visible = true;
        showStatus(id, { tone: "progress", title: copy.title, description: copy.description, testId: copy.testId });
      }, STATUS_TIMING.GRACE_MS);
    }
    return;
  }
  if (!t.active) return;
  t.active = false;
  if (t.grace) {
    window.clearTimeout(t.grace);
    t.grace = undefined;
  }
  if (!t.visible) return; // brief blip: nothing was shown, so no "Connected" either
  t.visible = false;
  if (!copy.successTitle) {
    clearStatus(id);
    return;
  }
  showStatus(id, { tone: "success", title: copy.successTitle, testId: copy.testId });
  t.success = window.setTimeout(() => {
    t.success = undefined;
    clearStatus(id);
  }, STATUS_TIMING.SUCCESS_MS);
}

/** Drop a tracked pill immediately (e.g. relay switched, component gone). */
export function resetProgress(id: string): void {
  const t = tracked.get(id);
  if (t?.grace) window.clearTimeout(t.grace);
  if (t?.success) window.clearTimeout(t.success);
  tracked.delete(id);
  clearStatus(id);
}
