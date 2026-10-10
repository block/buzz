// Desktop side of the bridge: publishes the key wrap, agent snapshots (on change + forced refresh) and heartbeats.
// IO is injected so it is testable and so the web build (which never runs it) pulls in nothing native.
import type { AgentsSnapshotBody, HeartbeatBody } from "./protocol";
import {
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
  private lastPublishAt = 0;
  private lastForcedAt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private hbTimer: ReturnType<typeof setInterval> | null = null;
  private agentsRunning = 0;
  private startedAt: number;
  private destroyed = false;
  readonly kid = this.key.kid;
  readonly stats = { snapshots: 0, heartbeats: 0, keyWraps: 0, skipped: 0 };

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
    this.timer = null;
    this.hbTimer = null;
  }

  /** Called whenever agent state may have changed; coalesces and rate-limits. */
  update(body: AgentsSnapshotBody) {
    if (this.destroyed) return;
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
    this.schedule();
  }

  private schedule() {
    if (this.timer) return;
    const sinceLast = this.nowMs() - this.lastPublishAt;
    const wait = Math.max(
      SNAPSHOT_DEBOUNCE_MS,
      SNAPSHOT_MIN_INTERVAL_MS - sinceLast,
    );
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
        caps: ["agents.snapshot"],
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
  ) {
    const content = encryptEnvelope(env, this.key.key);
    const signed = await this.io.signEvent({
      kind: BRIDGE_KIND,
      content,
      createdAt: env.ts,
      tags: bridgeTags(d, this.kid, env.ts + ttlS),
    });
    await this.io.publish(signed);
  }
}
