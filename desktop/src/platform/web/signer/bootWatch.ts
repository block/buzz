// Vanilla-DOM (outside React) diagnostics:
//  1. `?debug=1`: floating badge with pending signer calls + the last N calls (tap to expand).
//  2. Always: if the boot splash (data-testid=app-loading-gate) stays up > 6s, show a "Still starting…" hint with what the
//     signer is being asked and a Sign out button, so a stuck boot is never a silent dead end.
import { dbgEnabled, nip46Debug } from "./nip46Debug";
import { lastSignerNote, signerCalls, signerStats } from "./instrument";
import { signerStatus } from "./signerStatus";

const STUCK_MS = 6000;
let started = false;

function mk(tag: string, css: string, text?: string) {
  const e = document.createElement(tag);
  e.style.cssText = css;
  if (text) e.textContent = text;
  return e;
}
const BASE =
  "position:fixed;left:8px;right:8px;z-index:2147483646;background:#111e;color:#f5f5f5;border:1px solid #444;border-radius:12px;font:12px/1.4 ui-monospace,Menlo,monospace;padding:8px 10px;";

export function startBootWatch(signOut: () => Promise<void> | void) {
  if (started) return;
  started = true;
  (window as unknown as { __buzzSignerCalls?: unknown }).__buzzSignerCalls = signerCalls; // e2e/debug hook (read-only data)
  const debug = dbgEnabled();
  let gateSince: number | null = null;
  let hint: HTMLDivElement | null = null;
  let badge: HTMLDivElement | null = null;
  let open = false;

  const render = () => {
    const st = signerStatus.get();
    const pending = Math.max(st.pending, st.queued);
    const gate = !!document.querySelector('[data-testid="app-loading-gate"]');
    const now = Date.now();
    if (gate) gateSince ??= now;
    else gateSince = null;

    // stuck-boot hint
    if (gate && gateSince !== null && now - gateSince > STUCK_MS) {
      if (!hint) {
        hint = mk("div", `${BASE}top:calc(env(safe-area-inset-top) + 8px);font:14px/1.4 Inter,system-ui,sans-serif;text-align:center`) as HTMLDivElement;
        hint.setAttribute("data-testid", "boot-stuck-hint");
        document.body.appendChild(hint);
      }
      hint.textContent = "";
      const t = mk("div", "margin-bottom:8px");
      const what = st.labels.slice(0, 3).join(", ");
      t.textContent =
        pending > 0
          ? `Still starting… waiting for signer (${pending} pending${what ? `: ${what}` : ""})`
          : "Still starting… loading your workspace";
      hint.appendChild(t);
      const note = lastSignerNote();
      if (note)
        hint.appendChild(
          mk("div", "margin-bottom:8px;font:12px/1.4 ui-monospace,Menlo,monospace;opacity:.8", note),
        );
      const b = mk(
        "button",
        "background:#fff;color:#111;border:0;border-radius:8px;padding:8px 14px;font:600 14px Inter,system-ui,sans-serif",
        "Sign out",
      );
      b.setAttribute("data-testid", "boot-stuck-signout");
      b.onclick = async () => {
        try {
          await signOut();
        } finally {
          location.reload();
        }
      };
      hint.appendChild(b);
    } else if (hint) {
      hint.remove();
      hint = null;
    }

    // debug badge
    if (!debug) return;
    if (!badge) {
      badge = mk("div", `${BASE}bottom:calc(env(safe-area-inset-bottom) + 4px);left:auto;max-width:92vw;opacity:.92`) as HTMLDivElement;
      badge.setAttribute("data-testid", "signer-debug-badge");
      badge.onclick = () => {
        open = !open;
        render();
      };
      document.body.appendChild(badge);
    }
    const calls = signerCalls.get();
    const relays = Object.values(nip46Debug.get().relays);
    const stats = signerStats();
    const sec = (ms: number | null) => (ms === null ? "–" : `${(ms / 1000).toFixed(1)}s`);
    const head = `signer: ${pending} pending · relays ${relays.filter(Boolean).length}/${relays.length} · ${calls.length} calls · p50 ${sec(stats.p50)} p95 ${sec(stats.p95)}`;
    if (!open) {
      badge.textContent = head;
      return;
    }
    const depth = `queue (queued/in-flight): ${stats.depth.map((d) => `${d.name} ${d.queued}/${d.inflight}`).join(" · ")} · dropped ${stats.dropped} · boot ${stats.bootReady ? "ready" : "loading"}`;
    const oldest = stats.oldest
      ? `oldest: "${stats.oldest.label}" ${Math.round(stats.oldest.ageMs / 1000)}s (${stats.oldest.status}, ${["send", "auth/read", "settings", "background"][stats.oldest.prio]})`
      : "oldest: none pending";
    const rows = calls
      .slice(-14)
      .reverse()
      .map((c) => {
        const d = c.ms !== null ? `${c.ms}ms` : `${Math.round((now - c.startedAt) / 1000)}s…`;
        return `#${c.n} P${c.prio} ${c.method}${c.kind !== undefined ? `:${c.kind}` : ""} ${c.status} ${d} — ${c.label}${c.error ? ` [${c.error.slice(0, 60)}]` : ""}${c.caller !== "?" ? ` <${c.caller.slice(0, 60)}>` : ""}`;
      });
    const note = lastSignerNote();
    badge.textContent = [head, depth, oldest, ...(note ? [`note: ${note}`] : []), ...rows].join("\n");
    badge.style.whiteSpace = "pre-wrap";
    badge.style.maxHeight = "45vh";
    badge.style.overflow = "auto";
  };
  signerCalls.subscribe(render);
  signerStatus.subscribe(render);
  nip46Debug.subscribe(render);
  setInterval(render, 1000);
  const go = () => render();
  if (document.body) go();
  else addEventListener("DOMContentLoaded", go);
}
