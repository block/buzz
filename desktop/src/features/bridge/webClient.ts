// Web side of the bridge: pure(ish) state machine that ingests relay events and exposes agent/presence state.
// Signer cost: exactly one nip44 decrypt per key event (cached); everything else is local crypto.
import {
  BRIDGE_KIND,
  BRIDGE_TAG,
  D_AGENTS,
  D_HEARTBEAT,
  D_KEY,
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

export type BridgeState = {
  presence: DesktopPresence;
  /** Seconds since the desktop was last heard from (heartbeat or snapshot), null if never. */
  lastHeardTs: number | null;
  snapshot: AgentsSnapshotBody | null;
  snapshotTs: number | null;
  heartbeat: HeartbeatBody | null;
  keyState: "none" | "decrypting" | "ready" | "failed";
  versionTooNew: boolean;
};

export type WebClientIO = {
  decryptFromSelf(ciphertext: string): Promise<string>;
  cacheGet(eventId: string): string | null;
  cachePut(eventId: string, wrappedJson: string): void;
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
  };
  readonly stats = { signerDecrypts: 0, localDecrypts: 0, dropped: 0 };
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
    if (this.seen.size > 500) this.seen.clear();
    const d = tagValue(ev, "d");
    const exp = Number(tagValue(ev, "expiration"));
    if (Number.isFinite(exp) && exp > 0 && nowS() > exp) {
      this.stats.dropped++;
      return;
    }
    if (d === D_KEY) return this.ingestKey(ev);
    if (d !== D_AGENTS && d !== D_HEARTBEAT) return;
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
    if (!env) {
      this.stats.dropped++;
      return;
    }
    if (!envelopeLive(env) || !this.seq.accept(env)) {
      this.stats.dropped++;
      return;
    }
    if (d === D_AGENTS && env.type === "agents.snapshot") {
      const snapshotTs = env.ts;
      this.set({
        snapshot: env.body as AgentsSnapshotBody,
        snapshotTs,
        lastHeardTs: Math.max(this.state.lastHeardTs ?? 0, env.ts),
        presence: presenceOf(Math.max(this.state.lastHeardTs ?? 0, env.ts)),
      });
    } else if (d === D_HEARTBEAT && env.type === "presence.heartbeat") {
      const lastHeardTs = Math.max(this.state.lastHeardTs ?? 0, env.ts);
      this.set({
        heartbeat: env.body as HeartbeatBody,
        lastHeardTs,
        presence: presenceOf(lastHeardTs),
      });
    }
  }

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
