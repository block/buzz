import { ESCALATE_WAIT_MS, signerHealth } from "./signerHealth";
// Tiny vanilla-DOM "Waiting for your signer…" indicator, independent of the React app tree.
type St = { pending: number; authUrl: string | null; queued: number; labels: string[] };
let st: St = { pending: 0, authUrl: null, queued: 0, labels: [] };
const listeners = new Set<() => void>();
let el: HTMLDivElement | null = null;
let showTimer: ReturnType<typeof setTimeout> | undefined;

function render() {
  if (st.pending === 0 && st.queued === 0) {
    clearTimeout(showTimer);
    showTimer = undefined;
    el?.remove();
    el = null;
    return;
  }
  if (el || showTimer) return update();
  // Only show if the signer takes noticeably long (>1.5s), to avoid flicker; then stay up until nothing is pending.
  showTimer = setTimeout(() => {
    showTimer = undefined;
    if (st.pending === 0 && st.queued === 0) return;
    el = document.createElement("div");
    el.setAttribute("data-testid", "signer-waiting");
    el.setAttribute("role", "status");
    el.style.cssText =
      "position:fixed;left:50%;transform:translateX(-50%);bottom:calc(env(safe-area-inset-bottom) + 16px);z-index:2147483647;background:#111;color:#f5f5f5;border:1px solid #444;border-radius:12px;padding:10px 14px;font:14px/1.4 Inter,system-ui,sans-serif;box-shadow:0 4px 20px #0008;display:flex;gap:10px;align-items:center";
    document.body.appendChild(el);
    update();
  }, 1500);
}
function update() {
  if (!el) return;
  el.textContent = "";
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
    a.style.cssText = "color:#fff;font-weight:600;text-decoration:underline";
    el.appendChild(a);
  }
}
let escalateTimer: ReturnType<typeof setTimeout> | undefined;
// "Waiting for your signer…" that lasts too long (and isn't the user approving in the signer app) becomes the unresponsive state.
function watchEscalation() {
  const waiting = st.pending > 0 || st.queued > 0;
  if (!waiting || st.authUrl) {
    clearTimeout(escalateTimer);
    escalateTimer = undefined;
    return;
  }
  escalateTimer ??= setTimeout(() => {
    escalateTimer = undefined;
    if ((st.pending > 0 || st.queued > 0) && !st.authUrl) signerHealth.escalate("long-wait");
  }, ESCALATE_WAIT_MS);
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
