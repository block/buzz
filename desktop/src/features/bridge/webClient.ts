// Web side of the bridge: pure(ish) state machine that ingests relay events and exposes agent/presence state.
// Signer cost: exactly one nip44 decrypt per key event (cached); everything else is local crypto.
import {
  BRIDGE_KIND,
  BRIDGE_TAG,
  D_AGENTS,
  D_BROWSER_SESSIONS,
  D_SHOT_PREFIX,
  SHOT_MAX_CHUNKS,
  type BrowserSessionInfo,
  type BrowserSessionsBody,
  type ShotChunkBody,
  D_HEARTBEAT,
  D_KEY,
  D_RES_PREFIX,
  D_TX_PREFIX,
  WEB_MAX_ENTRIES_PER_AGENT,
  type CommandResultBody,
  type TranscriptChunkBody,
  type TranscriptEntry,
  decryptEnvelope,
  envelopeLive,
  keyFromHex,
  nowS,
  presenceOf,
  SeqGuard,
  tagValue,
  type AgentsSnapshotBody,
  type BridgeEventLike,
  type DesktopPresence,
  type HeartbeatBody,
  type WrappedKey,
} from "./protocol";

/** Keeps only known, well-typed fields of a published browser (the body is decrypted but still treated as untrusted). */
export function cleanSession(x: BrowserSessionInfo): BrowserSessionInfo {
  const str = (v: unknown, n: number) => (typeof v === "string" ? v.slice(0, n) : undefined);
  const a = x.agent as BrowserSessionInfo["agent"] | undefined;
  const url = str(x.url, 2048);
  return {
    sessionId: x.sessionId.slice(0, 200),
    title: str(x.title, 80) ?? "",
    host: x.host.slice(0, 253),
    openedAt: typeof x.openedAt === "number" ? x.openedAt : 0,
    ...(url && /^https?:\/\//i.test(url) ? { url } : {}),
    ...(x.status === "live" || x.status === "cold" ? { status: x.status } : {}),
    ...(x.origin === "web" || x.origin === "desktop" ? { origin: x.origin } : {}),
    ...(x.restricted === true ? { restricted: true } : {}),
    ...(a && typeof a.pubkey === "string" && (a.mode === "observe" || a.mode === "drive")
      ? { agent: { pubkey: a.pubkey.slice(0, 64), name: str(a.name, 60) ?? a.pubkey.slice(0, 8), mode: a.mode } }
      : {}),
  };
}

export type BridgeState = {
  presence: DesktopPresence;
  /** Seconds since the desktop was last heard from (heartbeat or snapshot), null if never. */
  lastHeardTs: number | null;
  snapshot: AgentsSnapshotBody | null;
  snapshotTs: number | null;
  heartbeat: HeartbeatBody | null;
  keyState: "none" | "decrypting" | "ready" | "failed";
  versionTooNew: boolean;
  /** agent pubkey -> entries (oldest first, merged by entry id, bounded). */
  transcripts: ReadonlyMap<string, readonly TranscriptEntry[]>;
  /** cmdId -> latest result seen from the desktop. */
  results: ReadonlyMap<string, CommandResultBody & { ts: number }>;
  /** Browser sessions the desktop currently shows remotely (slice R). */
  browserSessions: readonly BrowserSessionInfo[];
  /** sessionId -> latest assembled screenshot (data URL) with the command that produced it. */
  shots: ReadonlyMap<string, { cmdId: string; src: string; w: number; h: number; ts: number }>;
  /** True while showing the cached last-known agent list and no live snapshot has arrived yet. */
  refreshing: boolean;
};

export type WebClientIO = {
  decryptFromSelf(ciphertext: string): Promise<string>;
  cacheGet(eventId: string): string | null;
  cachePut(eventId: string, wrappedJson: string): void;
  /** Last known agent list (survives reloads) so the panel can draw instantly while the live data refreshes. */
  snapGet?(): { snapshot: AgentsSnapshotBody; ts: number } | null;
  snapPut?(snapshot: AgentsSnapshotBody, ts: number): void;
  /** Last known desktop heartbeat (survives reloads) so Settings/hosts know the desktop before the key wrap is decrypted. */
  hbGet?(): { heartbeat: HeartbeatBody; ts: number } | null;
  hbPut?(heartbeat: HeartbeatBody, ts: number): void;
};

export class BridgeWebClient {
  state: BridgeState = {
    presence: "unknown",
    lastHeardTs: null,
    snapshot: null,
    snapshotTs: null,
    heartbeat: null,
    keyState: "none",
    versionTooNew: false,
    transcripts: new Map(),
    results: new Map(),
    browserSessions: [],
    shots: new Map(),
    refreshing: false,
  };
  readonly stats = {
    signerDecrypts: 0,
    localDecrypts: 0,
    dropped: 0,
    /** debug ring: last applied/dropped payloads */
    log: [] as string[],
  };
  private note(s: string) {
    this.stats.log.push(s);
    if (this.stats.log.length > 30) this.stats.log.shift();
  }
  private listeners = new Set<() => void>();
  readonly keys = new Map<string, Uint8Array>(); // kid -> K
  private keyHeadAt = 0;
  private keyHeadId = "";
  private waitingByD = new Map<string, BridgeEventLike>();
  private seq = new SeqGuard();
  private seen = new Set<string>();

  private pubkey: string;
  private io: WebClientIO;
  constructor(pubkey: string, io: WebClientIO) {
    this.pubkey = pubkey;
    this.io = io;
    const cached = io.snapGet?.();
    if (cached)
      this.state = { ...this.state, snapshot: cached.snapshot, snapshotTs: cached.ts, lastHeardTs: cached.ts, presence: presenceOf(cached.ts), refreshing: true };
    const hb = io.hbGet?.();
    if (hb) {
      const lastHeardTs = Math.max(this.state.lastHeardTs ?? 0, hb.ts);
      this.state = { ...this.state, heartbeat: hb.heartbeat, lastHeardTs, presence: presenceOf(lastHeardTs) };
    }
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
  private set(patch: Partial<BridgeState>) {
    this.state = { ...this.state, ...patch };
    this.emit();
  }

  /** Recompute presence from the clock (call on a timer). */
  tick(now = nowS()) {
    const p = presenceOf(this.state.lastHeardTs, now);
    if (p !== this.state.presence) this.set({ presence: p });
  }

  async ingest(ev: BridgeEventLike): Promise<void> {
    if (ev.kind !== BRIDGE_KIND || ev.pubkey !== this.pubkey) return;
    if (tagValue(ev, "t") !== BRIDGE_TAG) return;
    if (this.seen.has(ev.id)) return;
    this.seen.add(ev.id);
    if (this.seen.size > 3000) this.seen.clear();
    const d = tagValue(ev, "d");
    const exp = Number(tagValue(ev, "expiration"));
    if (Number.isFinite(exp) && exp > 0 && nowS() > exp) {
      this.stats.dropped++;
      return;
    }
    if (d === D_KEY) return this.ingestKey(ev);
    if (
      d !== D_AGENTS &&
      d !== D_HEARTBEAT &&
      d !== D_BROWSER_SESSIONS &&
      !d?.startsWith(D_SHOT_PREFIX) &&
      !d?.startsWith(D_TX_PREFIX) &&
      !d?.startsWith(D_RES_PREFIX)
    )
      return;
    const kid = tagValue(ev, "kid");
    const key = kid ? this.keys.get(kid) : undefined;
    if (!kid || !key) {
      // Unknown key: park the newest event per d-tag until the matching key event arrives. No signer call here.
      const prev = this.waitingByD.get(d);
      if (!prev || ev.created_at >= prev.created_at) this.waitingByD.set(d, ev);
      return;
    }
    this.applyPayload(d, ev, key);
  }

  private applyPayload(d: string, ev: BridgeEventLike, key: Uint8Array) {
    this.stats.localDecrypts++;
    const env = decryptEnvelope(ev.content, key);
    this.note(`${d.slice(15)} ${env ? `${env.type}#${env.seq}` : "undecryptable"}`);
    if (!env) {
      this.stats.dropped++;
      return;
    }
    // Chunks and results are idempotent (merged by entry id / command id, deduped by event id), so only
    // snapshot/heartbeat use the monotonic seq guard.
    const guarded = d === D_AGENTS || d === D_HEARTBEAT || d === D_BROWSER_SESSIONS;
    if (!envelopeLive(env) || (guarded && !this.seq.accept(env))) {
      this.stats.dropped++;
      return;
    }
    if (d === D_BROWSER_SESSIONS && env.type === "browser.sessions") {
      const b = env.body as BrowserSessionsBody;
      if (b && Array.isArray(b.sessions))
        this.set({
          browserSessions: b.sessions
            .filter((x) => x && typeof x.sessionId === "string" && typeof x.host === "string")
            .slice(0, 10)
            .map(cleanSession),
        });
      return;
    }
    if (d.startsWith(D_SHOT_PREFIX) && env.type === "browser.shot") {
      const cmdId = d.slice(D_SHOT_PREFIX.length).split("/")[0];
      if (env.re !== cmdId) {
        this.stats.dropped++;
        return;
      }
      this.applyShot(cmdId, env.ts, env.body as ShotChunkBody);
      return;
    }
    if (d.startsWith(D_TX_PREFIX) && env.type === "transcript.chunk") {
      this.applyChunk(env.body as TranscriptChunkBody, env.seq);
      return;
    }
    if (d.startsWith(D_RES_PREFIX) && env.type === "cmd.result") {
      const cmdId = d.slice(D_RES_PREFIX.length);
      if (env.re !== cmdId) {
        this.stats.dropped++;
        return;
      }
      const body = env.body as CommandResultBody;
      if (!body || typeof body.status !== "string") return;
      const prev = this.state.results.get(cmdId);
      // never regress a final result to an ack
      if (prev && prev.status !== "accepted" && body.status === "accepted") return;
      const results = new Map(this.state.results);
      results.set(cmdId, { ...body, ts: env.ts });
      const patch: Partial<BridgeState> = { results };
      // Optimistic list update: once the desktop says a start/stop worked, show it now (the next real snapshot replaces this).
      const expect = this.expected.get(cmdId);
      if (expect && body.status === "ok") {
        this.expected.delete(cmdId);
        const snap = this.state.snapshot;
        if (snap)
          patch.snapshot = {
            ...snap,
            agents: snap.agents.map((a) => (a.pubkey.toLowerCase() === expect.pubkey ? { ...a, status: expect.status } : a)),
          };
      }
      this.set(patch);
      return;
    }
    if (d === D_AGENTS && env.type === "agents.snapshot") {
      const snapshotTs = env.ts;
      this.io.snapPut?.(env.body as AgentsSnapshotBody, snapshotTs);
      this.set({
        refreshing: false,
        snapshot: env.body as AgentsSnapshotBody,
        snapshotTs,
        lastHeardTs: Math.max(this.state.lastHeardTs ?? 0, env.ts),
        presence: presenceOf(Math.max(this.state.lastHeardTs ?? 0, env.ts)),
      });
    } else if (d === D_HEARTBEAT && env.type === "presence.heartbeat") {
      const lastHeardTs = Math.max(this.state.lastHeardTs ?? 0, env.ts);
      this.io.hbPut?.(env.body as HeartbeatBody, env.ts);
      this.set({
        heartbeat: env.body as HeartbeatBody,
        lastHeardTs,
        presence: presenceOf(lastHeardTs),
      });
    }
  }

  private expected = new Map<string, { pubkey: string; status: "running" | "stopped" }>();
  /** Remember what a start/stop command should leave behind, to show it as soon as the desktop confirms. */
  expectStatus(cmdId: string, pubkey: string, status: "running" | "stopped") {
    this.expected.set(cmdId, { pubkey: pubkey.toLowerCase(), status });
    if (this.expected.size > 50) this.expected.delete(this.expected.keys().next().value as string);
  }

  private shotParts = new Map<string, { n: number; parts: Map<number, string>; b: ShotChunkBody; ts: number }>();
  private applyShot(cmdId: string, ts: number, b: ShotChunkBody) {
    if (
      !b ||
      typeof b.sessionId !== "string" ||
      typeof b.data !== "string" ||
      !Number.isInteger(b.i) ||
      !Number.isInteger(b.n) ||
      b.n < 1 ||
      b.n > SHOT_MAX_CHUNKS ||
      b.i < 0 ||
      b.i >= b.n ||
      !/^image\/(jpeg|png|webp)$/.test(b.mime)
    )
      return;
    let acc = this.shotParts.get(cmdId);
    if (!acc || acc.n !== b.n) {
      acc = { n: b.n, parts: new Map(), b, ts };
      this.shotParts.set(cmdId, acc);
      if (this.shotParts.size > 8) this.shotParts.delete(this.shotParts.keys().next().value as string);
    }
    acc.parts.set(b.i, b.data);
    if (acc.parts.size < acc.n) return;
    this.shotParts.delete(cmdId);
    const prev = this.state.shots.get(b.sessionId);
    if (prev && prev.ts > ts) return;
    let src = `data:${b.mime};base64,`;
    for (let i = 0; i < acc.n; i++) src += acc.parts.get(i);
    const shots = new Map(this.state.shots);
    shots.set(b.sessionId, { cmdId, src, w: Number(b.w) || 0, h: Number(b.h) || 0, ts });
    this.set({ shots });
  }

  private entryRev = new Map<string, number>();
  private applyChunk(body: TranscriptChunkBody, seq: number) {
    if (
      !body ||
      typeof body.agentPubkey !== "string" ||
      !Array.isArray(body.entries)
    )
      return;
    const agent = body.agentPubkey.toLowerCase();
    const byId = new Map<string, TranscriptEntry>(
      (this.state.transcripts.get(agent) ?? []).map((e) => [e.id, e]),
    );
    for (const e of body.entries) {
      if (!e || typeof e.id !== "string" || typeof e.text !== "string") continue;
      const rk = `${agent}|${e.id}`;
      // chunks can arrive out of order: only a higher chunk seq may replace an entry
      if ((this.entryRev.get(rk) ?? -1) >= seq) continue;
      this.entryRev.set(rk, seq);
      byId.set(e.id, e);
    }
    const merged = [...byId.values()].sort((a, b) => a.t - b.t);
    const bounded = merged.slice(-WEB_MAX_ENTRIES_PER_AGENT);
    const transcripts = new Map(this.state.transcripts);
    transcripts.set(agent, bounded);
    this.set({ transcripts });
  }

  /** Current session key for building commands (null until the key event is unlocked). */
  currentKey(): { kid: string; key: Uint8Array } | null {
    const kid = this.keyHeadKid;
    const key = kid ? this.keys.get(kid) : undefined;
    return kid && key ? { kid, key } : null;
  }
  private keyHeadKid = "";

  private async ingestKey(ev: BridgeEventLike) {
    if (
      ev.created_at < this.keyHeadAt ||
      (ev.created_at === this.keyHeadAt && ev.id === this.keyHeadId)
    )
      return;
    this.keyHeadAt = ev.created_at;
    this.keyHeadId = ev.id;
    let json = this.io.cacheGet(ev.id);
    if (!json) {
      this.set({ keyState: "decrypting" });
      try {
        this.stats.signerDecrypts++;
        json = await this.io.decryptFromSelf(ev.content);
        this.io.cachePut(ev.id, json);
      } catch {
        if (this.keyHeadId === ev.id) this.set({ keyState: "failed" });
        return;
      }
    }
    if (this.keyHeadId !== ev.id) return; // superseded while decrypting
    try {
      const w = JSON.parse(json) as WrappedKey;
      if (w.v !== 1 || typeof w.key !== "string" || nowS() > w.expiresAt)
        throw new Error("bad key");
      this.keys.set(w.kid, keyFromHex(w.key));
      this.keyHeadKid = w.kid;
    } catch {
      this.set({ keyState: "failed" });
      return;
    }
    this.set({ keyState: "ready" });
    for (const [d, e] of [...this.waitingByD]) {
      const k = this.keys.get(tagValue(e, "kid") ?? "");
      if (k) {
        this.waitingByD.delete(d);
        this.applyPayload(d, e, k);
      }
    }
  }
}
