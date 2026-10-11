// Desktop-side command intake for the bridge (docs/pwa-bridge-protocol.md §6). Pure: all IO is injected.
// A command is executed only if EVERY check passes; anything else is dropped (silently to the sender, loudly to onDrop).
import { verifyEvent } from "nostr-tools";
import { chunkImage, validateBrowserUrl, type BrowserSessionManager } from "./browserSessions";
import {
  ALL_COMMANDS,
  BRIDGE_KIND,
  BRIDGE_TAG,
  CMD_MAX_TEXT,
  CMD_MAX_TTL_S,
  CMD_SEEN_LIMIT,
  D_CMD_PREFIX,
  MAX_FUTURE_SKEW_S,
  decryptEnvelope,
  nowS,
  tagValue,
  type BridgeEventLike,
  type CommandResultBody,
  type CommandType,
} from "./protocol";

export type KnownAgent = { status: "running" | "stopped" | "deployed" | "not_deployed" };

/** Slice R: the desktop's built-in browser sessions (pinned sites), view-only from the web. */
export type BrowserDeps = {
  manager: BrowserSessionManager;
  /** Adds the link as a browser session in the desktop client and returns its id. */
  open(url: string, host: string): Promise<{ sessionId: string; title: string }>;
  close(sessionId: string): Promise<void>;
  /** One fresh, compressed image of the session (already size-capped by the caller's encoder). */
  capture(sessionId: string): Promise<{ mime: string; w: number; h: number; bytes: Uint8Array }>;
  publishShot(
    cmdId: string,
    shot: { sessionId: string; mime: string; w: number; h: number; chunks: Uint8Array[] },
  ): Promise<void>;
  publishSessions(): Promise<void>;
};

export type CommandDeps = {
  /** The member's own pubkey (hex, lowercase). */
  memberPubkey: string;
  /** Current session key; commands under any other kid are rejected. */
  sessionKey(): { kid: string; key: Uint8Array };
  /** Agents from the latest published snapshot (lowercase pubkey -> status). */
  agents(): ReadonlyMap<string, KnownAgent>;
  seen: { has(id: string): boolean; add(id: string): void };
  executors: {
    start(agentPubkey: string): Promise<string | void>;
    stop(agentPubkey: string): Promise<string | void>;
    message(agentPubkey: string, channelId: string, text: string): Promise<string | void>;
  };
  publishResult(cmdId: string, body: CommandResultBody): Promise<void>;
  browser?: BrowserDeps;
  /** "Allow remote agent management" on this desktop. Absent = allowed (tests); when false agent.* commands are refused. */
  agentManagementAllowed?(): boolean;
  onDrop?(reason: string, eventId: string): void;
  now?(): number; // unix seconds
  /** Overridable for tests; defaults to nostr-tools verifyEvent (id hash + schnorr signature). */
  verify?(ev: BridgeEventLike): boolean;
};

export type HandleOutcome = "executed" | "replayed" | "dropped";

/** In-memory (optionally persisted) bounded id set. */
export function makeSeenSet(
  load: () => string[],
  save: (ids: string[]) => void,
  limit = CMD_SEEN_LIMIT,
) {
  let ids = load().slice(-limit);
  const set = new Set(ids);
  return {
    has: (id: string) => set.has(id),
    add(id: string) {
      if (set.has(id)) return;
      set.add(id);
      ids.push(id);
      if (ids.length > limit) {
        const drop = ids.splice(0, ids.length - limit);
        for (const d of drop) set.delete(d);
      }
      save(ids);
    },
  };
}

const HEX64 = /^[0-9a-f]{64}$/;

export class BridgeCommandHandler {
  private results = new Map<string, CommandResultBody>();
  readonly stats = { executed: 0, replayed: 0, dropped: 0 };
  private deps: CommandDeps;
  constructor(deps: CommandDeps) {
    this.deps = deps;
  }

  private drop(reason: string, id: string): HandleOutcome {
    this.stats.dropped++;
    this.deps.onDrop?.(reason, id);
    return "dropped";
  }

  private async finish(cmdId: string, res: CommandResultBody) {
    this.results.set(cmdId, res);
    if (this.results.size > 300) this.results.delete(this.results.keys().next().value as string);
    await this.deps.publishResult(cmdId, res);
  }

  private async handleBrowser(
    type: CommandType,
    cmdId: string,
    body: Record<string, unknown>,
  ): Promise<HandleOutcome> {
    const d = this.deps;
    const b = d.browser;
    d.seen.add(cmdId);
    if (!b) {
      await this.finish(cmdId, { status: "error", code: "unsupported", message: "Browser sessions aren't available on this desktop." });
      return "dropped";
    }
    try {
      if (type === "browser.open") {
        const v = validateBrowserUrl(body.url);
        if (!v.ok) await this.finish(cmdId, { status: "error", code: v.code, message: v.message });
        else if (!b.manager.canOpen())
          await this.finish(cmdId, { status: "error", code: "session_limit", message: "Too many remote browser sessions are open. Close one first." });
        else {
          await d.publishResult(cmdId, { status: "accepted" });
          const { sessionId, title } = await b.open(v.url, v.host);
          b.manager.add({ sessionId, title, host: v.host, openedAt: (d.now ?? nowS)() });
          await b.publishSessions();
          await this.finish(cmdId, { status: "ok", result: { sessionId } });
        }
      } else if (type === "browser.close") {
        const id = typeof body.sessionId === "string" ? body.sessionId : "";
        if (!b.manager.has(id)) await this.finish(cmdId, { status: "error", code: "unknown_session", message: "That browser session isn't open remotely." });
        else {
          await d.publishResult(cmdId, { status: "accepted" });
          await b.close(id);
          b.manager.remove(id);
          await b.publishSessions();
          await this.finish(cmdId, { status: "ok" });
        }
      } else {
        const id = typeof body.sessionId === "string" ? body.sessionId : "";
        if (!b.manager.has(id)) {
          await this.finish(cmdId, { status: "error", code: "unknown_session", message: "That browser session isn't open remotely." });
        } else {
          const gate = b.manager.beginShot(id, Date.now());
          if (gate) {
            await this.finish(cmdId, {
              status: "error",
              code: gate,
              message: gate === "busy" ? "A screenshot is already being taken." : "Please wait a few seconds before another screenshot.",
            });
          } else {
            try {
              await d.publishResult(cmdId, { status: "accepted" });
              const shot = await b.capture(id);
              const chunks = chunkImage(shot.bytes);
              if (!chunks) await this.finish(cmdId, { status: "error", code: "too_large", message: "The screenshot is too large to send." });
              else {
                await b.publishShot(cmdId, { sessionId: id, mime: shot.mime, w: shot.w, h: shot.h, chunks });
                await this.finish(cmdId, { status: "ok", result: { chunks: chunks.length } });
              }
            } finally {
              b.manager.endShot();
            }
          }
        }
      }
    } catch (e) {
      await this.finish(cmdId, { status: "error", code: "failed", message: String(e instanceof Error ? e.message : e).slice(0, 200) });
    }
    this.stats.executed++;
    return "executed";
  }

  async handle(ev: BridgeEventLike): Promise<HandleOutcome> {
    const d = this.deps;
    const now = (d.now ?? nowS)();
    if (ev.kind !== BRIDGE_KIND || tagValue(ev, "t") !== BRIDGE_TAG) return "dropped"; // not ours: not even counted
    const dTag = tagValue(ev, "d") ?? "";
    if (!dTag.startsWith(D_CMD_PREFIX)) return "dropped";
    const cmdId = dTag.slice(D_CMD_PREFIX.length);
    // 1. authorship + signature: only the member's own key may command the desktop
    if (ev.pubkey.toLowerCase() !== d.memberPubkey) return this.drop("foreign-author", ev.id);
    if (!(d.verify ?? ((e) => verifyEvent(e as never)))(ev)) return this.drop("bad-signature", ev.id);
    // 2. event freshness (cheap pre-filter before any crypto)
    if (Math.abs(now - ev.created_at) > CMD_MAX_TTL_S + MAX_FUTURE_SKEW_S) return this.drop("stale-event", ev.id);
    const exp = Number(tagValue(ev, "expiration"));
    if (Number.isFinite(exp) && exp > 0 && now > exp) return this.drop("expired-tag", ev.id);
    // 3. key: must be under the CURRENT session key
    const cur = d.sessionKey();
    if (tagValue(ev, "kid") !== cur.kid) return this.drop("unknown-kid", ev.id);
    const env = decryptEnvelope(ev.content, cur.key);
    if (!env) return this.drop("undecryptable", ev.id);
    if (env.kid !== cur.kid) return this.drop("kid-mismatch", ev.id);
    // 4. envelope validity
    if (env.id !== cmdId) return this.drop("id-mismatch", ev.id);
    if (!/^[0-9a-zA-Z-]{8,64}$/.test(cmdId)) return this.drop("bad-id", ev.id);
    if (now > env.exp) return this.drop("expired", ev.id);
    if (env.ts > now + MAX_FUTURE_SKEW_S) return this.drop("future", ev.id);
    if (env.exp - env.ts > CMD_MAX_TTL_S) return this.drop("ttl-too-long", ev.id);
    if (!(ALL_COMMANDS as readonly string[]).includes(env.type)) return this.drop("unknown-type", ev.id);
    const type = env.type as CommandType;
    // 5. replay / idempotency
    const stored = this.results.get(cmdId);
    if (stored) {
      this.stats.replayed++;
      await d.publishResult(cmdId, stored);
      return "replayed";
    }
    if (d.seen.has(cmdId)) return this.drop("replayed-unknown", ev.id); // seen in a previous run: never re-execute
    // 6. body validation + known agent
    const body = (env.body ?? {}) as Record<string, unknown>;
    if (type.startsWith("browser.")) return this.handleBrowser(type, cmdId, body);
    if (type.startsWith("agent.") && d.agentManagementAllowed && !d.agentManagementAllowed()) {
      d.seen.add(cmdId);
      const res: CommandResultBody = { status: "error", code: "remote_disabled", message: "Remote agent management is off on this computer. Turn on \"Allow remote agent management\" in its Settings → Agents." };
      this.results.set(cmdId, res);
      await d.publishResult(cmdId, res);
      return "dropped";
    }
    const agentPubkey = typeof body.agentPubkey === "string" ? body.agentPubkey.toLowerCase() : "";
    const agent = HEX64.test(agentPubkey) ? d.agents().get(agentPubkey) : undefined;
    d.seen.add(cmdId); // claim the id before any await so a concurrent duplicate can't double-run
    const finish = async (res: CommandResultBody) => {
      this.results.set(cmdId, res);
      if (this.results.size > 300) this.results.delete(this.results.keys().next().value as string);
      await d.publishResult(cmdId, res);
    };
    if (!agent) {
      await finish({ status: "error", code: "unknown_agent", message: "Not one of your desktop's agents." });
      return "dropped";
    }
    await d.publishResult(cmdId, { status: "accepted" });
    try {
      if (type === "agent.start") {
        if (agent.status === "running") await finish({ status: "ok", message: "Already running." });
        else {
          const m = await d.executors.start(agentPubkey);
          await finish({ status: "ok", ...(m ? { message: m } : {}) });
        }
      } else if (type === "agent.stop") {
        if (agent.status !== "running") await finish({ status: "ok", message: "Already stopped." });
        else {
          const m = await d.executors.stop(agentPubkey);
          await finish({ status: "ok", ...(m ? { message: m } : {}) });
        }
      } else if (type === "agent.message") {
        const channelId = typeof body.channelId === "string" ? body.channelId : "";
        const text = typeof body.text === "string" ? body.text : "";
        if (!channelId || channelId.length > 200 || !text.trim() || text.length > CMD_MAX_TEXT)
          await finish({ status: "error", code: "bad_request", message: "Invalid channel or message text." });
        else if (agent.status !== "running")
          await finish({ status: "error", code: "agent_not_running", message: "Start the agent first." });
        else {
          await d.executors.message(agentPubkey, channelId, text);
          await finish({ status: "ok" });
        }
      } else {
        // prompt.approve / prompt.deny: this desktop has no interactive permission prompts to answer.
        await finish({ status: "error", code: "unsupported", message: "This desktop does not support answering permission prompts remotely." });
      }
    } catch (e) {
      await finish({ status: "error", code: "failed", message: String(e instanceof Error ? e.message : e).slice(0, 200) });
    }
    this.stats.executed++;
    return "executed";
  }
}
