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

export function installProxyHandlers(handlers: Record<string, (a?: any) => unknown>) {
  for (const name of CMD_NAMES) {
    const empty = name in PROXIED_READS ? PROXIED_READS[name] : null;
    handlers[name] = (a) => (empty && !proxyDirectLive() ? empty() : proxyCmd(name, a));
  }
}
