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

export async function proxyCmd<T = unknown>(name: string, args?: Record<string, unknown>): Promise<T> {
  const d = current;
  if (!d || !d.live) throw new NeedsDirectError();
  try {
    return (await slot(() => d.call("cmd", { name, args: args ?? {} }))) as T;
  } catch (e) {
    throw new Error(directErrorCopy(e));
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
};

/** Commands the web can answer itself: prefer the desktop's answer when Direct is live, else (or if that call fails) the web's own. */
const PREFER_DESKTOP = new Set(["get_presence"]);

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
    handlers[name] = (a) => (empty && !proxyDirectLive() ? empty() : proxyCmd(name, a));
  }
}
