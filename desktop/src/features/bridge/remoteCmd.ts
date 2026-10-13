// Web side of the desktop command proxy. The web app runs the desktop's own Agents UI; its data commands are answered by the
// desktop over the direct (tailnet) connection (directCmd.ts executes them there). Without a live direct connection reads
// return the web's empty default (the page says why) and writes fail with NeedsDirectError, so the web app's own local
// values are never shown or edited as if they were the desktop's.
import { CMD_NAMES } from "./directCmd";
import { type DirectAgents, directErrorCopy } from "./directAgents";

export const NEEDS_DIRECT = "Needs a Direct connection over your tailnet: turn on Allow remote agent management on your desktop and connect this device to Tailscale.";
export class NeedsDirectError extends Error {
  code = "needs_direct";
  constructor() {
    super(NEEDS_DIRECT);
    this.name = "NeedsDirectError";
  }
}

let current: DirectAgents | null = null;
const listeners = new Set<() => void>();
export function setProxyDirect(d: DirectAgents | null) {
  if (current === d) return;
  current = d;
  for (const l of listeners) l();
}
export const getProxyDirect = () => current;
// Several places (left nav, Settings) bind the same shared client; it stays bound until the last of them goes away.
const holds = new Map<DirectAgents, number>();
/** Binds `d` as the proxy's direct connection; returns a release function. Safe to call from any number of components. */
export function acquireProxyDirect(d: DirectAgents | null): () => void {
  if (!d) return () => {};
  holds.set(d, (holds.get(d) ?? 0) + 1);
  setProxyDirect(d);
  let done = false;
  return () => {
    if (done) return;
    done = true;
    const n = (holds.get(d) ?? 1) - 1;
    if (n > 0) return void holds.set(d, n);
    holds.delete(d);
    if (current === d) setProxyDirect(null);
  };
}
export const proxyDirectLive = () => !!current?.live;
/**
 * User-initiated actions from any screen (e.g. a term-session card's Open in a channel): the left-nav watcher binds the shared
 * Direct client app-wide but only the Agents/Settings pages connect it. Connect it now (or wait for a connect in progress) so the
 * command doesn't fail with NeedsDirectError while the desktop is reachable. Resolves true once live, false after `timeoutMs`.
 */
export async function ensureProxyDirectLive(timeoutMs = 8000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (!current && Date.now() < deadline) await new Promise((r) => setTimeout(r, 200));
  const d = current;
  if (!d) return false;
  if (d.live) return true;
  if (d.state !== "connecting") void d.connect().catch(() => undefined);
  while (Date.now() < deadline) {
    if (current?.live) return true;
    await new Promise((r) => setTimeout(r, 150));
  }
  return !!current?.live;
}
export function subscribeProxy(fn: () => void) {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}

// The desktop accepts 4 requests in flight; keep headroom for the page's own calls.
const MAX_INFLIGHT = 3;
let inflight = 0;
const waiting: Array<() => void> = [];
async function slot<T>(f: () => Promise<T>): Promise<T> {
  if (inflight >= MAX_INFLIGHT) await new Promise<void>((r) => waiting.push(r));
  inflight++;
  try {
    return await f();
  } finally {
    inflight--;
    waiting.shift()?.();
  }
}

/** Commands the connected desktop answered with unknown_op (it runs an older HulaBuzz). Cleared on the next success. */
const staleCmds = new Set<string>();
export const isCmdStale = (name: string) => staleCmds.has(name);

export async function proxyCmd<T = unknown>(name: string, args?: Record<string, unknown>): Promise<T> {
  // Any screen may call before the shared Direct client is connected (e.g. an agent's profile Runtime tab): connect on demand
  // instead of failing straight away.
  if (!current?.live) await ensureProxyDirectLive();
  const d = current;
  if (!d || !d.live) throw new NeedsDirectError();
  try {
    const v = (await slot(() => d.call("cmd", { name, args: args ?? {} }))) as T;
    staleCmds.delete(name);
    return v;
  } catch (e) {
    if ((e as { code?: string })?.code === "unknown_op") staleCmds.add(name);
    const err = new Error(directErrorCopy(e)) as Error & { code?: string };
    err.code = (e as { code?: string })?.code;
    throw err;
  }
}

/** Desktop command names answered over Direct. Reads fall back to `empty` without a connection; writes throw. */
export const PROXIED_READS: Record<string, () => unknown> = {
  list_managed_agents: () => [],
  list_managed_agent_runtimes: () => [],
  list_personas: () => [],
  discover_acp_providers: () => [],
  agent_access_owner_only: () => false,
  get_global_agent_config: () => ({ env_vars: {}, provider: null, model: null, preferred_runtime: null }),
  // Project pages: quiet defaults while the desktop isn't reachable (the repo card explains why).
  gh_pr_list: () => "[]",
  get_git_identity: () => ({ name: null, email: null }),
};

/** Commands the web can answer itself: prefer the desktop's answer when Direct is live, else (or if that call fails) the web's own. */
const PREFER_DESKTOP = new Set(["get_presence"]);

const CACHE_PREFIX = "buzz-desktop-cache:";
const memCache = new Map<string, unknown>();
const cacheKey = (name: string, a: unknown) => `${name}:${JSON.stringify(a ?? {})}`;
function readCache(key: string): unknown {
  if (memCache.has(key)) return memCache.get(key);
  try {
    const raw = localStorage.getItem(CACHE_PREFIX + key);
    if (raw === null) return undefined;
    const v = JSON.parse(raw);
    memCache.set(key, v);
    return v;
  } catch {
    return undefined;
  }
}
function writeCache(key: string, v: unknown) {
  memCache.set(key, v);
  try {
    localStorage.setItem(CACHE_PREFIX + key, JSON.stringify(v)); // already sanitized by the desktop (no secrets)
  } catch {
    /* storage full or denied */
  }
}
/**
 * Last agent list the desktop published over the relay (the bridge snapshot, saved on every snapshot), shaped like the desktop's
 * own list so the desktop-style Agents page can draw before any Direct connection. Read-only fields only.
 */
function latestRelaySnapshot(): { ts: number; agents: Record<string, unknown>[] } | undefined {
  let best: { ts: number; agents: Record<string, unknown>[] } | undefined;
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (!k?.startsWith("buzz-bridge-lastlist:")) continue;
      const v = JSON.parse(localStorage.getItem(k) || "null");
      const agents = v?.snapshot?.agents;
      const ts = Number(v?.ts ?? 0);
      if (Array.isArray(agents) && (!best || ts > best.ts)) best = { ts, agents };
    }
  } catch {
    /* storage denied */
  }
  return best;
}

/**
 * Offline list: the last full Direct answer with each agent's name and live status taken from the newest relay snapshot
 * (it is republished on every desktop change, so it is fresher than the Direct cache).
 */
function overlaySnapshot(list: unknown): unknown {
  const snap = latestRelaySnapshot();
  if (!Array.isArray(list) || !snap) return list;
  const by = new Map(snap.agents.map((a) => [String(a.pubkey).toLowerCase(), a]));
  return list.map((row: Record<string, unknown>) => {
    const a = by.get(String(row.pubkey).toLowerCase());
    return a ? { ...row, name: a.name ?? row.name, status: a.status ?? row.status, pid: a.pid ?? row.pid, last_error: a.lastError ?? null } : row;
  });
}

export function relaySnapshotAgents(): unknown[] | undefined {
  try {
    {
      const v0 = latestRelaySnapshot();
      if (!v0) return undefined;
      const v = { ts: v0.ts };
      const agents = v0.agents;
      const t = new Date((v.ts ?? 0) * 1000).toISOString();
      return agents.map((a: Record<string, unknown>) => ({
        pubkey: a.pubkey, name: a.name, persona_id: a.personaId ?? null, runtime: a.runtime ?? null, team_id: null, relay_url: "",
        acp_command: "", agent_command: "", agent_args: [], mcp_command: "", turn_timeout_seconds: 0, idle_timeout_seconds: null,
        max_turn_duration_seconds: null, parallelism: 1, system_prompt: null, avatar_url: null, model: a.model ?? null, provider: null,
        persona_out_of_date: false, persona_orphaned: false, needs_restart: false, env_vars: {}, status: a.status, pid: a.pid ?? null,
        created_at: t, updated_at: t, last_started_at: a.lastStartedAt ?? null, last_stopped_at: null, last_exit_code: null,
        last_error: a.lastError ?? null, last_error_code: null, log_path: "", start_on_app_launch: false, backend: { type: "local" },
        backend_agent_id: null,
      }));
    }
  } catch {
    /* storage denied */
  }
  return undefined;
}

/** True when this device has a remembered desktop agent list (the desktop-style Agents page can draw without Direct). */
export function hasCachedDesktopAgents(): boolean {
  const v = readCache(cacheKey("list_managed_agents", undefined));
  return Array.isArray(v) || !!relaySnapshotAgents()?.length;
}

export function installProxyHandlers(handlers: Record<string, (a?: any) => unknown>) {
  for (const name of CMD_NAMES) {
    if (PREFER_DESKTOP.has(name)) {
      const own = handlers[name];
      handlers[name] = async (a) => {
        if (proxyDirectLive()) {
          try {
            return await proxyCmd(name, a);
          } catch {
            /* fall through to the web's own answer */
          }
        }
        return own ? own(a) : {};
      };
      continue;
    }
    const empty = name in PROXIED_READS ? PROXIED_READS[name] : null;
    if (!empty) {
      handlers[name] = (a) => proxyCmd(name, a);
      continue;
    }
    // Reads: the desktop's answer when Direct is live (remembered), else the last known answer (so the desktop-style pages keep
    // showing your agents while reconnecting), else empty.
    handlers[name] = async (a) => {
      const key = cacheKey(name, a);
      if (!proxyDirectLive()) {
        const cached = readCache(key);
        if (name === "list_managed_agents") return (cached !== undefined ? overlaySnapshot(cached) : relaySnapshotAgents()) ?? empty();
        return cached ?? empty();
      }
      const v = await proxyCmd(name, a);
      writeCache(key, v);
      return v;
    };
  }
}
