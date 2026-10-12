// Web-side command sender (docs/pwa-bridge-protocol.md §6). Destructive commands refuse to send without an explicit
// `confirmed: true`, so a UI bug can't fire them silently.
import {
  BRIDGE_KIND,
  CMD_MAX_TTL_S,
  DESTRUCTIVE_COMMANDS,
  bridgeTags,
  buildEnvelope,
  cmdD,
  encryptEnvelope,
  nowS,
  type CommandBody,
  type CommandType,
} from "./protocol";
import type { BridgeWebClient } from "./webClient";

export type SentCommand = {
  id: string;
  type: CommandType;
  agentPubkey: string;
  label: string;
  ts: number;
  exp: number;
};

export type CommanderIO = {
  signEvent(e: { kind: number; content: string; createdAt: number; tags: string[][] }): Promise<unknown>;
  publish(signed: unknown): Promise<void>;
};

export class CommandError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export class BridgeCommander {
  sent: ReadonlyMap<string, SentCommand> = new Map();
  private listeners = new Set<() => void>();
  private seq = 0;
  private client: BridgeWebClient;
  private io: CommanderIO;
  private deviceId: string;
  constructor(client: BridgeWebClient, io: CommanderIO, deviceId: string) {
    this.client = client;
    this.io = io;
    this.deviceId = deviceId;
  }
  subscribe(fn: () => void) {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }
  private emit() {
    for (const l of this.listeners) l();
  }

  /** Whether the connected desktop advertises this command. */
  supports(type: CommandType): boolean {
    return (this.client.state.heartbeat?.caps ?? []).includes(type);
  }

  /** pending | accepted | ok | error | timeout for a sent command. */
  statusOf(id: string, now = nowS()): "pending" | "accepted" | "ok" | "error" | "timeout" {
    const r = this.client.state.results.get(id);
    if (r && r.status !== "accepted") return r.status;
    const s = this.sent.get(id);
    if (s && now > s.exp) return "timeout";
    return r ? "accepted" : "pending";
  }

  /** True while a screenshot request is still waiting on the desktop (the web UI allows one at a time). */
  shotBusy(now = nowS()): boolean {
    for (const s of this.sent.values()) {
      if (s.type !== "browser.screenshot") continue;
      const st = this.statusOf(s.id, now);
      if (st === "pending" || st === "accepted") return true;
    }
    return false;
  }

  async send(
    type: CommandType,
    body: CommandBody,
    opts: { confirmed?: boolean; label?: string } = {},
  ): Promise<string> {
    if ((DESTRUCTIVE_COMMANDS as readonly string[]).includes(type) && opts.confirmed !== true)
      throw new CommandError("confirmation_required", "This action needs an explicit confirmation.");
    if (type === "browser.screenshot" && this.shotBusy())
      throw new CommandError("busy", "A screenshot is already on its way.");
    if (!this.supports(type))
      throw new CommandError("unsupported", "Your desktop doesn't support this action (update the desktop app).");
    if (this.client.state.presence !== "online")
      throw new CommandError("offline", "Your desktop is offline.");
    const k = this.client.currentKey();
    if (!k) throw new CommandError("no_key", "The bridge key isn't unlocked yet.");
    const env = buildEnvelope({
      type,
      kid: k.kid,
      dev: `web-${this.deviceId}`,
      seq: ++this.seq,
      ttlS: CMD_MAX_TTL_S,
      body,
    });
    const signed = await this.io.signEvent({
      kind: BRIDGE_KIND,
      content: encryptEnvelope(env, k.key),
      createdAt: env.ts,
      tags: bridgeTags(cmdD(env.id), k.kid, env.exp),
    });
    const next = new Map(this.sent);
    next.set(env.id, {
      id: env.id,
      type,
      agentPubkey: (body as { agentPubkey?: string }).agentPubkey ?? "",
      label: opts.label ?? type,
      ts: env.ts,
      exp: env.exp,
    });
    this.sent = next;
    this.emit();
    if (type === "agent.start" || type === "agent.stop") {
      const pk = (body as { agentPubkey?: string }).agentPubkey;
      if (pk) this.client.expectStatus(env.id, pk, type === "agent.start" ? "running" : "stopped");
    }
    await this.io.publish(signed);
    return env.id;
  }
}
