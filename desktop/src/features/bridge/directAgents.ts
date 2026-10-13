// Web side of agent management over the direct (tailnet) connection. One signed connection is reused for its whole
// lifetime (the desktop's "Signature lifetime"), so the signer is asked once, not per action. Any failure to reach the
// desktop directly is reported as a RemoteError so callers can fall back to the relay (create/rename/edit have no relay path).
import { AutoReconnector } from "./autoReconnect";
import { onResume } from "@/shared/lib/resumeSignals";
import { RemoteConnection, RemoteError, type SignJson, type WsFactory } from "./remoteClient";

export type DirectAgent = {
  pubkey: string;
  name: string;
  status: "running" | "stopped" | "deployed" | "not_deployed";
  runtime?: string;
  model?: string;
  provider?: string;
  personaId?: string;
  pid?: number;
  lastStartedAt?: string;
  lastError?: string;
};
export type DirectRuntime = { id: string; label: string; available: boolean };
export type DirectPersona = { id: string; name: string };
export type AgentEdit = { name?: string; provider?: string | null; model?: string | null; runtimeId?: string };
export type NewAgent = { name: string; provider?: string; model?: string; personaId?: string; runtimeId?: string; start?: boolean };

export type AgentMutation = { op: "start" | "stop" | "create" | "rename" | "update"; pubkey?: string; name?: string };

export type DirectState = "idle" | "connecting" | "connected" | "failed";

/** Errors that mean "the direct path isn't usable right now": fall back to the relay where one exists. */
export const DIRECT_DOWN = new Set(["unreachable", "timeout", "closed", "session_expired", "bad_url", "idle_timeout"]);
export const isDirectDown = (e: unknown) => e instanceof RemoteError && DIRECT_DOWN.has(e.code);

export class DirectAgents {
  state: DirectState = "idle";
  /** Bumped on every change (for useSyncExternalStore). */
  version = 0;
  lastError: RemoteError | null = null;
  private conn: RemoteConnection | null = null;
  private connecting: Promise<RemoteConnection> | null = null;
  private listeners = new Set<() => void>();
  private info: { desktopId: string; url: string };
  private sign: SignJson;
  private makeWs?: WsFactory;
  private now: () => number;
  constructor(info: { desktopId: string; url: string }, sign: SignJson, makeWs?: WsFactory, now: () => number = Date.now) {
    this.info = info;
    this.sign = sign;
    this.makeWs = makeWs;
    this.now = now;
  }

  /** Retries a dropped connection with backoff (1 s → 30 s, forever while visible); stops on success or a decline. */
  readonly auto = new AutoReconnector(
    async () => {
      if (this.lastError?.code === "declined") {
        this.auto.stop();
        return false;
      }
      try {
        await this.connect();
        return true;
      } catch (e) {
        if (e instanceof RemoteError && e.code === "declined") this.auto.stop();
        return false;
      }
    },
    () => {
      this.version++;
      for (const l of this.listeners) l();
    },
  );
  /** Unsubscribe the resume probe (tests). */
  readonly stopResumeProbe = onResume(() => this.conn?.probe());
  /** True while a dropped connection is being brought back automatically. */
  get reconnecting(): boolean {
    return this.auto.active;
  }

  subscribe(fn: () => void) {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  }
  private set(state: DirectState, err: RemoteError | null = null) {
    this.state = state;
    this.version++;
    this.lastError = err;
    for (const l of this.listeners) l();
  }

  /** True while a signed connection is open and its signature hasn't expired. */
  get live(): boolean {
    return !!this.conn?.ready && (this.conn.expiresAt === 0 || this.now() < this.conn.expiresAt);
  }
  /** When the current signature ends (ms), or 0. */
  get expiresAt(): number {
    return this.live ? (this.conn?.expiresAt ?? 0) : 0;
  }

  /** Connects (asks the signer once) unless a live signed connection already exists. */
  async connect(): Promise<RemoteConnection> {
    if (this.live && this.conn) return this.conn;
    this.connecting ??= (async () => {
      this.set("connecting");
      const c = new RemoteConnection(this.info, this.sign, this.makeWs);
      c.onClose = () => {
        if (this.conn === c) {
          this.conn = null;
          this.set("idle");
          this.auto.start(); // dropped or went silent: come back on our own
        }
      };
      try {
        await c.connect();
      } catch (e) {
        const err = e instanceof RemoteError ? e : new RemoteError(/reject|denied|declin/i.test(String(e)) ? "declined" : "failed");
        this.set("failed", err);
        if (err.code !== "declined") this.auto.start(); // unreachable/failed: keep trying on our own (never after a decline)
        throw err;
      }
      this.conn = c;
      this.set("connected");
      return c;
    })().finally(() => {
      this.connecting = null;
    });
    return this.connecting;
  }

  private autoTried = false;
  /** Automatic connect for when the Agents page opens: at most one attempt per page visit (no retry loop after a decline or failure). */
  async autoConnect(): Promise<void> {
    if (this.live || this.connecting || this.autoTried) return;
    this.autoTried = true;
    try {
      await this.connect();
    } catch {
      /* the badge shows the reason and a Retry control; the relay keeps working */
    }
  }
  /** Reconnect on its own after the connection dropped (heartbeat back, window refocused) — never after the user declined. */
  async autoReconnect(): Promise<void> {
    if (this.live || this.connecting) return;
    if (this.lastError?.code === "declined") return;
    this.autoTried = false;
    await this.autoConnect();
  }
  /** Called when the Agents page closes, so the next visit may connect automatically again. */
  resetAuto() {
    this.autoTried = false;
  }

  close() {
    this.auto.stop();
    const c = this.conn;
    this.conn = null; // detach first so the deliberate close doesn't trigger auto-reconnect
    c?.close();
    this.set("idle");
  }

  /** One request. Reconnects once (a new signature) if the signed connection ended meanwhile. */
  async call<T = Record<string, unknown>>(op: string, params: Record<string, unknown> = {}): Promise<T> {
    for (let attempt = 0; ; attempt++) {
      const c = await this.connect();
      try {
        return await c.mgmt<T>(op, params);
      } catch (e) {
        const again = attempt === 0 && e instanceof RemoteError && (e.code === "closed" || e.code === "session_expired");
        if (!again) throw e;
        if (this.conn === c) this.conn = null;
      }
    }
  }

  async list(): Promise<DirectAgent[]> {
    return (await this.call<{ agents: DirectAgent[] }>("agents.list")).agents ?? [];
  }
  private mutationListeners = new Set<(m: AgentMutation) => void>();
  /** Fires after every acknowledged change to agent state made through this connection (any component), so lists can update at once. */
  onMutation(fn: (m: AgentMutation) => void) {
    this.mutationListeners.add(fn);
    return () => void this.mutationListeners.delete(fn);
  }
  private async mutate<T>(m: AgentMutation, p: Promise<T>): Promise<T> {
    const r = await p;
    for (const l of this.mutationListeners) l(m);
    return r;
  }
  start = (pubkey: string) => this.mutate({ op: "start", pubkey }, this.call("agent.start", { pubkey }));
  stop = (pubkey: string) => this.mutate({ op: "stop", pubkey }, this.call("agent.stop", { pubkey }));
  message = (pubkey: string, channelId: string, text: string) => this.call("agent.message", { pubkey, channelId, text });
  create = (a: NewAgent) => this.mutate({ op: "create" }, this.call<{ agent: DirectAgent; spawnError?: string }>("agent.create", { ...a }));
  rename = (pubkey: string, name: string) => this.mutate({ op: "rename", pubkey, name }, this.call<{ agent: DirectAgent }>("agent.rename", { pubkey, name }));
  update = (pubkey: string, edit: AgentEdit) =>
    this.mutate({ op: "update", pubkey }, this.call<{ agent: DirectAgent; restartNeeded?: boolean }>("agent.update", { pubkey, ...edit }));
  async runtimes(): Promise<DirectRuntime[]> {
    return (await this.call<{ runtimes: DirectRuntime[] }>("providers.list")).runtimes ?? [];
  }
  async personas(): Promise<DirectPersona[]> {
    return (await this.call<{ personas: DirectPersona[] }>("personas.list")).personas ?? [];
  }
}

// One client per desktop id for the whole tab, so the Agents page, Settings and the controls share the signed connection.
const clients = new Map<string, DirectAgents>();
export function directAgentsFor(info: { desktopId: string; url: string }, sign: SignJson): DirectAgents {
  const key = `${info.desktopId}|${info.url}`;
  let c = clients.get(key);
  if (!c) {
    c = new DirectAgents(info, sign);
    clients.set(key, c);
  }
  return c;
}

/** Human copy for a failed direct call. */
export function directErrorCopy(e: unknown): string {
  if (!(e instanceof RemoteError)) return e instanceof Error ? e.message : String(e);
  switch (e.code) {
    case "agents_disabled":
      return 'Remote agent management is off on that computer (Settings → Agents → Allow remote agent management).';
    case "declined":
    case "user_rejected":
      return "Approval was declined in your signer.";
    case "unreachable":
    case "timeout":
    case "closed":
      return "Couldn't reach your desktop directly. Connect to your tailnet to use create, rename and edit.";
    case "session_expired":
      return "The signed connection expired. Connect again to continue.";
    case "busy":
      return "Your desktop is busy with other requests. Try again in a moment.";
    case "unknown_op":
      return "Reload your desktop to see this (it's running an older version of the app).";
    default:
      return e.detail || `The desktop refused the request (${e.code}).`;
  }
}
