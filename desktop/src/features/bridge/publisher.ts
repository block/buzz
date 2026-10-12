// Desktop side of the bridge: publishes the key wrap, agent snapshots (on change + forced refresh) and heartbeats.
// IO is injected so it is testable and so the web build (which never runs it) pulls in nothing native.
import { bytesToBase64 } from "./browserSessions";
import type {
  BrowserSessionInfo,
  RemoteInfo,
  BrowserSessionsBody,
  ShotChunkBody,
  AgentSummary,
  AgentsSnapshotBody,
  CommandResultBody,
  HeartbeatBody,
  TranscriptChunkBody,
  TranscriptEntry,
} from "./protocol";
import {
  D_BROWSER_SESSIONS,
  SHOT_TTL_S,
  shotD,
  CMD_RESULT_TTL_S,
  resD,
  TX_DEBOUNCE_MS,
  TX_MAX_EVENTS_PER_MIN,
  TX_MIN_INTERVAL_MS,
  TX_TTL_S,
  txChunkD,
  bridgeTags,
  buildEnvelope,
  D_AGENTS,
  D_HEARTBEAT,
  D_KEY,
  encryptEnvelope,
  generateSessionKey,
  HEARTBEAT_INTERVAL_MS,
  HEARTBEAT_TTL_S,
  KEY_TTL_S,
  keyToHex,
  nowS,
  SNAPSHOT_DEBOUNCE_MS,
  SNAPSHOT_FORCE_INTERVAL_MS,
  SNAPSHOT_MIN_INTERVAL_MS,
  SNAPSHOT_TTL_S,
  BRIDGE_KIND,
  type WrappedKey,
} from "./protocol";
import { snapshotSignature } from "./snapshot";
import { packChunks, TranscriptTracker } from "./transcript";

/** Coalesce window for an agent lifecycle change (start/stop/create/rename/runtime status) before it is published. */
export const URGENT_SNAPSHOT_DELAY_MS = 250;

/** The part of a snapshot that is agent lifecycle state (not the "working in #channel" activity). */
export function agentStateSignature(b: AgentsSnapshotBody): string {
  return JSON.stringify(b.agents.map((a) => [a.pubkey, a.name, a.status, a.pid ?? null, a.runtime ?? null, a.model ?? null]));
}

export const DEFAULT_CAPS = ["agents.snapshot", "transcript.chunk"];

export type PublisherIO = {
  encryptToSelf(plaintext: string): Promise<string>;
  signEvent(e: {
    kind: number;
    content: string;
    createdAt: number;
    tags: string[][];
  }): Promise<unknown>;
  publish(signed: unknown): Promise<void>;
  now?: () => number; // ms
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
};

export type PublisherOptions = {
  deviceId: string;
  appVersion?: string;
  platform?: string;
  /** Capabilities advertised in the heartbeat (the web hides controls the desktop doesn't list). */
  caps?: string[];
  /** Slice B listener state, read on every heartbeat. */
  remote?: () => RemoteInfo | undefined;
  onError?: (e: unknown) => void;
};

export class BridgePublisher {
  private key = generateSessionKey();
  private seq = { agents: 0, hb: 0 };
  private started = false;
  private keyPublished = false;
  private keyPromise: Promise<void> | null = null;
  private pending: AgentsSnapshotBody | null = null;
  private lastSig: string | null = null;
  private lastStateSig: string | null = null;
  private timerUrgent = false;
  private lastPublishAt = 0;
  private lastForcedAt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private hbTimer: ReturnType<typeof setInterval> | null = null;
  private agentsRunning = 0;
  private startedAt: number;
  private destroyed = false;
  private knownAgents = new Set<string>();
  private agentStatus = new Map<string, { status: AgentSummary["status"] }>();
  /** Latest known agents and status (for command validation). */
  agents(): ReadonlyMap<string, { status: AgentSummary["status"] }> {
    return this.agentStatus;
  }
  private tx = new Map<
    string,
    {
      tracker: TranscriptTracker;
      pending: TranscriptEntry[] | null;
      timer: ReturnType<typeof setTimeout> | null;
      lastAt: number;
      seq: number;
    }
  >();
  private txSentAt: number[] = [];
  readonly kid = this.key.kid;
  /** Current session key, for the command handler's decrypt. */
  get sessionKey(): Uint8Array {
    return this.key.key;
  }
  readonly stats = {
    snapshots: 0,
    heartbeats: 0,
    keyWraps: 0,
    skipped: 0,
    txChunks: 0,
    txThrottled: 0,
  };

  private io: PublisherIO;
  private opts: PublisherOptions;

  constructor(io: PublisherIO, opts: PublisherOptions) {
    this.io = io;
    this.opts = opts;
    this.startedAt = this.nowMs();
  }

  private nowMs() {
    return (this.io.now ?? Date.now)();
  }

  start() {
    if (this.started || this.destroyed) return;
    this.started = true;
    void this.sendHeartbeat();
    this.hbTimer = setInterval(
      () => void this.sendHeartbeat(),
      HEARTBEAT_INTERVAL_MS,
    );
  }

  destroy() {
    this.destroyed = true;
    if (this.timer) (this.io.clearTimeout ?? clearTimeout)(this.timer);
    if (this.hbTimer) clearInterval(this.hbTimer);
    for (const t of this.tx.values())
      if (t.timer) (this.io.clearTimeout ?? clearTimeout)(t.timer);
    this.timer = null;
    this.hbTimer = null;
  }

  /** Called whenever agent state may have changed; coalesces and rate-limits. */
  update(body: AgentsSnapshotBody) {
    if (this.destroyed) return;
    this.knownAgents = new Set(body.agents.map((a) => a.pubkey.toLowerCase()));
    this.agentStatus = new Map(
      body.agents.map((a) => [a.pubkey.toLowerCase(), { status: a.status }]),
    );
    this.agentsRunning = body.agents.filter(
      (a) => a.status === "running",
    ).length;
    const sig = snapshotSignature(body);
    const forced =
      this.nowMs() - this.lastForcedAt >= SNAPSHOT_FORCE_INTERVAL_MS;
    if (sig === this.lastSig && !forced) {
      this.stats.skipped++;
      return;
    }
    this.pending = body;
    // Start/stop/create/rename/runtime-status changes go out almost immediately (a short coalesce), not on the slow timer;
    // churn like "working in #channel" keeps the normal debounce and rate limit.
    const stateSig = agentStateSignature(body);
    const urgent = this.lastStateSig !== null && stateSig !== this.lastStateSig;
    this.lastStateSig = stateSig;
    if (urgent) {
      if (this.timer && !this.timerUrgent) {
        (this.io.clearTimeout ?? clearTimeout)(this.timer);
        this.timer = null;
      }
      this.schedule(URGENT_SNAPSHOT_DELAY_MS);
      return;
    }
    this.schedule();
  }

  private schedule(fixedWait?: number) {
    if (this.timer) return;
    this.timerUrgent = fixedWait !== undefined;
    const sinceLast = this.nowMs() - this.lastPublishAt;
    const wait =
      fixedWait ??
      Math.max(SNAPSHOT_DEBOUNCE_MS, SNAPSHOT_MIN_INTERVAL_MS - sinceLast);
    this.timer = (this.io.setTimeout ?? setTimeout)(() => {
      this.timer = null;
      void this.flush();
    }, wait);
  }

  async flush() {
    const body = this.pending;
    if (!body || this.destroyed) return;
    this.pending = null;
    try {
      await this.ensureKey();
      const env = buildEnvelope({
        type: "agents.snapshot",
        kid: this.kid,
        dev: this.opts.deviceId,
        seq: ++this.seq.agents,
        ttlS: SNAPSHOT_TTL_S,
        body,
      });
      await this.publishEnv(D_AGENTS, env, SNAPSHOT_TTL_S);
      this.lastSig = snapshotSignature(body);
      this.lastPublishAt = this.nowMs();
      this.lastForcedAt = this.lastPublishAt;
      this.stats.snapshots++;
    } catch (e) {
      this.opts.onError?.(e);
      this.pending ??= body; // retry on next update/forced refresh
    }
  }

  async sendHeartbeat() {
    if (this.destroyed) return;
    try {
      await this.ensureKey();
      const body: HeartbeatBody = {
        bridge: "v1",
        appVersion: this.opts.appVersion,
        platform: this.opts.platform,
        agentsRunning: this.agentsRunning,
        uptimeSec: Math.floor((this.nowMs() - this.startedAt) / 1000),
        caps: this.opts.caps ?? DEFAULT_CAPS,
        ...(this.opts.remote?.() ? { remote: this.opts.remote() } : {}),
      };
      const env = buildEnvelope({
        type: "presence.heartbeat",
        kid: this.kid,
        dev: this.opts.deviceId,
        seq: ++this.seq.hb,
        ttlS: HEARTBEAT_TTL_S,
        body,
      });
      await this.publishEnv(D_HEARTBEAT, env, HEARTBEAT_TTL_S);
      this.stats.heartbeats++;
    } catch (e) {
      this.opts.onError?.(e);
    }
  }

  /** True if this agent was in the latest snapshot (only known agents' transcripts are published). */
  knowsAgent(pubkey: string): boolean {
    return this.knownAgents.has(pubkey.toLowerCase());
  }

  /**
   * Feed the agent's current redacted transcript entries (all of them; the tracker sends only new/changed ones).
   * Debounced per agent, and globally capped to TX_MAX_EVENTS_PER_MIN chunk events.
   */
  updateTranscript(agentPubkey: string, entries: TranscriptEntry[]) {
    if (this.destroyed) return;
    const agent = agentPubkey.toLowerCase();
    if (!this.knownAgents.has(agent)) return;
    let st = this.tx.get(agent);
    if (!st) {
      st = { tracker: new TranscriptTracker(), pending: null, timer: null, lastAt: 0, seq: 0 };
      this.tx.set(agent, st);
    }
    const delta = st.tracker.delta(entries);
    if (delta.length === 0) return;
    st.pending = entries;
    this.scheduleTx(agent);
  }

  private scheduleTx(agent: string, extraWait = 0) {
    const st = this.tx.get(agent);
    if (!st || st.timer) return;
    const wait = Math.max(
      TX_DEBOUNCE_MS,
      TX_MIN_INTERVAL_MS - (this.nowMs() - st.lastAt),
      extraWait,
    );
    st.timer = (this.io.setTimeout ?? setTimeout)(() => {
      st.timer = null;
      void this.flushTranscript(agent);
    }, wait);
  }

  async flushTranscript(agent: string) {
    const st = this.tx.get(agent);
    if (!st || !st.pending || this.destroyed) return;
    const now = this.nowMs();
    this.txSentAt = this.txSentAt.filter((t) => now - t < 60_000);
    const delta = st.tracker.delta(st.pending);
    if (delta.length === 0) {
      st.pending = null;
      return;
    }
    const chunks = packChunks(agent, delta);
    if (this.txSentAt.length + chunks.length > TX_MAX_EVENTS_PER_MIN) {
      this.stats.txThrottled++;
      this.scheduleTx(agent, 10_000); // retry later; pending is kept
      return;
    }
    try {
      await this.ensureKey();
      for (const entries of chunks) {
        const body: TranscriptChunkBody = { agentPubkey: agent, entries };
        const env = buildEnvelope({
          type: "transcript.chunk",
          kid: this.kid,
          dev: this.opts.deviceId,
          seq: ++st.seq,
          ttlS: TX_TTL_S,
          body,
        });
        await this.publishEnv(txChunkD(agent, st.seq), env, TX_TTL_S);
        this.txSentAt.push(this.nowMs());
        this.stats.txChunks++;
        st.tracker.markSent(entries);
      }
      st.lastAt = this.nowMs();
      // Newer changes may have arrived while publishing.
      const again = st.pending ? st.tracker.delta(st.pending) : [];
      if (again.length === 0) st.pending = null;
      else this.scheduleTx(agent);
    } catch (e) {
      this.opts.onError?.(e);
      this.scheduleTx(agent, 15_000);
    }
  }

  /** Publish one screenshot as encrypted chunk events (one request -> one image; replaceable per command/chunk). */
  async publishShot(
    cmdId: string,
    shot: { sessionId: string; mime: string; w: number; h: number; chunks: Uint8Array[] },
  ) {
    await this.ensureKey();
    const n = shot.chunks.length;
    for (let i = 0; i < n; i++) {
      const body: ShotChunkBody = {
        sessionId: shot.sessionId,
        mime: shot.mime,
        w: shot.w,
        h: shot.h,
        i,
        n,
        data: bytesToBase64(shot.chunks[i]),
      };
      const env = buildEnvelope({
        type: "browser.shot",
        kid: this.kid,
        dev: this.opts.deviceId,
        seq: Date.now() + i,
        ttlS: SHOT_TTL_S,
        body,
        re: cmdId,
      });
      await this.publishEnv(shotD(cmdId, i), env, SHOT_TTL_S);
    }
  }

  /** Publish the list of remotely visible browser sessions (replaceable snapshot). */
  async publishBrowserSessions(sessions: BrowserSessionInfo[]) {
    if (this.destroyed) return;
    try {
      await this.ensureKey();
      const env = buildEnvelope({
        type: "browser.sessions",
        kid: this.kid,
        dev: this.opts.deviceId,
        seq: Date.now(),
        ttlS: SNAPSHOT_TTL_S,
        body: { sessions } satisfies BrowserSessionsBody,
      });
      const prev = this.sessionsAt;
      const createdAt = Math.max(env.ts, prev + 1);
      this.sessionsAt = createdAt;
      await this.publishEnv(D_BROWSER_SESSIONS, env, SNAPSHOT_TTL_S, createdAt);
    } catch (e) {
      this.opts.onError?.(e);
    }
  }
  private sessionsAt = 0;

  private resultCreatedAt = new Map<string, number>();

  /** Publish an ack/result for a command (replaceable per command id; ack -> final). */
  async publishResult(cmdId: string, body: CommandResultBody) {
    if (this.destroyed) return;
    try {
      await this.ensureKey();
      const env = buildEnvelope({
        type: "cmd.result",
        kid: this.kid,
        dev: this.opts.deviceId,
        seq: Date.now(),
        ttlS: CMD_RESULT_TTL_S,
        body,
        re: cmdId,
      });
      const prev = this.resultCreatedAt.get(cmdId) ?? 0;
      const createdAt = Math.max(env.ts, prev + 1); // replaceable ordering: final must sort after the ack
      this.resultCreatedAt.set(cmdId, createdAt);
      if (this.resultCreatedAt.size > 500) this.resultCreatedAt.clear();
      await this.publishEnv(resD(cmdId), env, CMD_RESULT_TTL_S, createdAt);
    } catch (e) {
      this.opts.onError?.(e);
    }
  }

  private ensureKey(): Promise<void> {
    if (this.keyPublished) return Promise.resolve();
    this.keyPromise ??= this.publishKey().then(
      () => {
        this.keyPublished = true;
      },
      (e) => {
        this.keyPromise = null;
        throw e;
      },
    );
    return this.keyPromise;
  }

  private async publishKey() {
    const createdAt = nowS();
    const wrapped: WrappedKey = {
      v: 1,
      kid: this.kid,
      key: keyToHex(this.key.key),
      createdAt,
      expiresAt: createdAt + KEY_TTL_S,
    };
    const content = await this.io.encryptToSelf(JSON.stringify(wrapped));
    const signed = await this.io.signEvent({
      kind: BRIDGE_KIND,
      content,
      createdAt,
      tags: bridgeTags(D_KEY, null, wrapped.expiresAt),
    });
    await this.io.publish(signed);
    this.stats.keyWraps++;
  }

  private async publishEnv(
    d: string,
    env: ReturnType<typeof buildEnvelope>,
    ttlS: number,
    createdAt = env.ts,
  ) {
    const content = encryptEnvelope(env, this.key.key);
    const signed = await this.io.signEvent({
      kind: BRIDGE_KIND,
      content,
      createdAt,
      tags: bridgeTags(d, this.kid, env.ts + ttlS),
    });
    await this.io.publish(signed);
  }
}
