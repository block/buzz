// Web: answer managed-agent permission prompts over Direct. Only agents launched with BUZZ_ACP_PERMISSION_MODE=ask on a
// desktop that has `list_permission_prompts` raise them; an older desktop answers unknown_op once and polling stops for the
// session (no UI, no errors). The desktop redacts titles/details before they leave it.
import { proxyCmd, proxyDirectLive, relaySnapshotAgents } from "@/features/bridge/remoteCmd";

export type Prompt = {
  id: string;
  title: string;
  kind: string;
  detail: string;
  createdAt: number;
  options: { optionId: string; name: string; kind: string }[];
};
type Pending = Prompt & { pubkey: string; agentName: string };

const POLL_MS = 4000;
let unsupported = false;
const answered = new Set<string>();

export function runningAgents(list: unknown): { pubkey: string; name: string }[] {
  if (!Array.isArray(list)) return [];
  return list
    .filter((a) => a && (a as { status?: string }).status === "running")
    .map((a) => ({ pubkey: String((a as { pubkey: string }).pubkey).toLowerCase(), name: String((a as { name?: string }).name ?? "Agent") }))
    .slice(0, 20);
}

/** Order: allow options first, then reject; labels fall back to the option kind. */
export function buttonsFor(p: Prompt): { optionId: string; label: string; allow: boolean }[] {
  return [...p.options]
    .sort((a, b) => Number(b.kind.startsWith("allow")) - Number(a.kind.startsWith("allow")))
    .map((o) => ({ optionId: o.optionId, label: o.name || o.kind.replace(/_/g, " "), allow: o.kind.startsWith("allow") }));
}

async function poll(): Promise<Pending[]> {
  if (unsupported || !proxyDirectLive() || document.visibilityState !== "visible") return [];
  const out: Pending[] = [];
  for (const a of runningAgents(relaySnapshotAgents())) {
    try {
      const list = await proxyCmd<Prompt[]>("list_permission_prompts", { pubkey: a.pubkey });
      for (const p of list ?? []) if (!answered.has(`${a.pubkey}/${p.id}`)) out.push({ ...p, pubkey: a.pubkey, agentName: a.name });
    } catch (e) {
      if ((e as { code?: string }).code === "unknown_op") {
        unsupported = true;
        return [];
      }
    }
  }
  return out;
}

let root: HTMLDivElement | null = null;
let shown = "";
function render(items: Pending[]) {
  const key = items.slice(0, 3).map((p) => `${p.pubkey}/${p.id}`).join(",");
  if (key === shown && (root || !items.length)) return; // unchanged: keep buttons (and any click in flight) as they are
  shown = key;
  if (!items.length) {
    root?.remove();
    root = null;
    return;
  }
  if (!root) {
    root = document.createElement("div");
    root.setAttribute("data-testid", "web-permission-prompts");
    root.style.cssText =
      "position:fixed;left:50%;transform:translateX(-50%);top:calc(env(safe-area-inset-top,0px) + 12px);z-index:2147483646;width:min(92vw,440px);display:flex;flex-direction:column;gap:8px;font:14px/1.35 system-ui,sans-serif";
    document.body.appendChild(root);
  }
  root.replaceChildren(
    ...items.slice(0, 3).map((p) => {
      const card = document.createElement("div");
      card.setAttribute("data-testid", "web-permission-prompt");
      card.style.cssText = "background:hsl(var(--popover,0 0% 100%));color:hsl(var(--popover-foreground,0 0% 10%));border:1px solid hsl(var(--border,0 0% 85%));border-radius:12px;padding:12px;box-shadow:0 6px 24px rgba(0,0,0,.18)";
      const h = document.createElement("div");
      h.style.fontWeight = "600";
      h.textContent = `${p.agentName} wants to: ${p.title}`;
      card.appendChild(h);
      if (p.detail) {
        const d = document.createElement("pre");
        d.style.cssText = "margin:6px 0 0;max-height:120px;overflow:auto;white-space:pre-wrap;font:12px ui-monospace,monospace;opacity:.8";
        d.textContent = p.detail;
        card.appendChild(d);
      }
      const row = document.createElement("div");
      row.style.cssText = "display:flex;gap:8px;margin-top:10px;flex-wrap:wrap";
      for (const b of buttonsFor(p)) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.textContent = b.label;
        btn.setAttribute("data-option", b.optionId);
        btn.style.cssText = `padding:6px 12px;border-radius:8px;border:1px solid hsl(var(--border,0 0% 80%));cursor:pointer;${b.allow ? "background:hsl(var(--primary,220 90% 50%));color:hsl(var(--primary-foreground,0 0% 100%))" : "background:transparent;color:inherit"}`;
        btn.onclick = async () => {
          row.querySelectorAll("button").forEach((x) => ((x as HTMLButtonElement).disabled = true));
          try {
            await proxyCmd("respond_permission_prompt", { pubkey: p.pubkey, requestId: p.id, optionId: b.optionId });
            answered.add(`${p.pubkey}/${p.id}`);
            card.remove();
          } catch (e) {
            row.querySelectorAll("button").forEach((x) => ((x as HTMLButtonElement).disabled = false));
            h.textContent = `${p.agentName}: ${(e as Error).message || "Couldn't send the answer."}`;
          }
        };
        row.appendChild(btn);
      }
      card.appendChild(row);
      return card;
    }),
  );
}

let started = false;
export function startPermissionPrompts() {
  if (started) return;
  started = true;
  let busy = false;
  const tick = async () => {
    if (busy) return;
    busy = true;
    try {
      render(await poll());
    } finally {
      busy = false;
    }
  };
  setInterval(() => void tick(), POLL_MS);
  document.addEventListener("visibilitychange", () => void tick());
}
