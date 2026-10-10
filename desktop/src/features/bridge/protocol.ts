// Buzz bridge protocol v1 (see docs/pwa-bridge-protocol.md). Pure module: no React, no Tauri, no relay IO.
import { nip44 } from "nostr-tools";
import { bytesToHex, hexToBytes } from "nostr-tools/utils";
import { sha256 } from "@noble/hashes/sha2.js";

export const BRIDGE_KIND = 30078;
export const BRIDGE_VERSION = 1;
export const BRIDGE_TAG = "buzz-bridge";
export const D_PREFIX = "buzz.bridge.v1/";
export const D_KEY = `${D_PREFIX}key`;
export const D_AGENTS = `${D_PREFIX}agents`;
export const D_HEARTBEAT = `${D_PREFIX}hb`;

export const HEARTBEAT_INTERVAL_MS = 30_000;
/** Heartbeats live 24 h so a long-offline desktop still shows "offline · last seen …"; freshness is judged by `ts` vs OFFLINE_AFTER_S, not by exp. */
export const HEARTBEAT_TTL_S = 24 * 3600;
/** Desktop is shown offline when the freshest heartbeat/snapshot is older than this. */
export const OFFLINE_AFTER_S = 90;
export const SNAPSHOT_TTL_S = 24 * 3600;
export const SNAPSHOT_MIN_INTERVAL_MS = 5_000;
export const SNAPSHOT_DEBOUNCE_MS = 2_000;
export const SNAPSHOT_FORCE_INTERVAL_MS = 5 * 60_000;
export const KEY_TTL_S = 7 * 24 * 3600;
export const MAX_FUTURE_SKEW_S = 5 * 60;
export const MAX_SNAPSHOT_AGENTS = 200;

export type AgentStatus = "running" | "stopped" | "deployed" | "not_deployed";

export type AgentSummary = {
  pubkey: string;
  name: string;
  status: AgentStatus;
  pid?: number;
  runtime?: string;
  model?: string;
  personaId?: string;
  lastStartedAt?: string;
  lastError?: string;
  activeTurns: { channelId: string; channelName?: string; since: number }[];
  currentTask?: string;
};

export type AgentsSnapshotBody = {
  agents: AgentSummary[];
  truncated?: boolean;
  generatedAt: number;
};

export type HeartbeatBody = {
  bridge: "v1";
  appVersion?: string;
  platform?: string;
  agentsRunning: number;
  uptimeSec: number;
  caps: string[];
};

export type BridgeType = "agents.snapshot" | "presence.heartbeat";

export type Envelope<B = unknown> = {
  v: number;
  type: string;
  kid: string;
  dev: string;
  seq: number;
  ts: number;
  exp: number;
  id: string;
  re?: string;
  body: B;
};

export type WrappedKey = {
  v: 1;
  kid: string;
  key: string;
  createdAt: number;
  expiresAt: number;
};

export type BridgeEventLike = {
  id: string;
  pubkey: string;
  created_at: number;
  kind: number;
  content: string;
  tags: string[][];
};

export const nowS = () => Math.floor(Date.now() / 1000);

export function tagValue(
  ev: { tags: string[][] },
  name: string,
): string | undefined {
  return ev.tags.find((t) => t[0] === name)?.[1];
}

export function generateSessionKey(): { key: Uint8Array; kid: string } {
  const key = new Uint8Array(32);
  crypto.getRandomValues(key);
  return { key, kid: keyIdOf(key) };
}

export function keyIdOf(key: Uint8Array): string {
  return bytesToHex(sha256(key)).slice(0, 16);
}

export const keyToHex = (k: Uint8Array) => bytesToHex(k);
export const keyFromHex = (h: string) => hexToBytes(h);

export function encryptEnvelope(env: Envelope, key: Uint8Array): string {
  return nip44.v2.encrypt(JSON.stringify(env), key);
}

export function decryptEnvelope(
  content: string,
  key: Uint8Array,
): Envelope | null {
  try {
    return validateEnvelope(JSON.parse(nip44.v2.decrypt(content, key)));
  } catch {
    return null;
  }
}

export function validateEnvelope(x: unknown): Envelope | null {
  if (!x || typeof x !== "object") return null;
  const e = x as Record<string, unknown>;
  if (typeof e.v !== "number" || e.v > BRIDGE_VERSION || e.v < 1) return null;
  if (
    typeof e.type !== "string" ||
    typeof e.kid !== "string" ||
    typeof e.dev !== "string"
  )
    return null;
  if (
    typeof e.seq !== "number" ||
    typeof e.ts !== "number" ||
    typeof e.exp !== "number" ||
    typeof e.id !== "string"
  )
    return null;
  if (!("body" in e)) return null;
  return e as unknown as Envelope;
}

/** Receiver-side freshness: not expired, not from the far future. */
export function envelopeLive(env: Envelope, now = nowS()): boolean {
  return now <= env.exp && env.ts <= now + MAX_FUTURE_SKEW_S;
}

export function buildEnvelope<B>(p: {
  type: BridgeType | string;
  kid: string;
  dev: string;
  seq: number;
  ttlS: number;
  body: B;
  id?: string;
  re?: string;
  now?: number;
}): Envelope<B> {
  const ts = p.now ?? nowS();
  return {
    v: BRIDGE_VERSION,
    type: p.type,
    kid: p.kid,
    dev: p.dev,
    seq: p.seq,
    ts,
    exp: ts + p.ttlS,
    id: p.id ?? crypto.randomUUID(),
    ...(p.re ? { re: p.re } : {}),
    body: p.body,
  };
}

export function bridgeTags(
  d: string,
  kid: string | null,
  exp: number,
): string[][] {
  return [
    ["d", d],
    ["t", BRIDGE_TAG],
    ...(kid ? [["kid", kid]] : []),
    ["expiration", String(exp)],
  ];
}

/** Per-(device,key-id,type) monotonic seq guard (replay / out-of-order). */
export class SeqGuard {
  private last = new Map<string, number>();
  accept(env: Envelope): boolean {
    const k = `${env.dev}|${env.kid}|${env.type}`;
    const prev = this.last.get(k);
    if (prev !== undefined && env.seq <= prev) return false;
    this.last.set(k, env.seq);
    return true;
  }
}

export type DesktopPresence = "unknown" | "online" | "offline";

/** Seconds since the desktop was last heard from, or null if never. */
export function lastHeardAgeS(
  lastTs: number | null,
  now = nowS(),
): number | null {
  return lastTs === null ? null : Math.max(0, now - lastTs);
}

export function presenceOf(
  lastTs: number | null,
  now = nowS(),
): DesktopPresence {
  if (lastTs === null) return "unknown";
  return now - lastTs > OFFLINE_AFTER_S ? "offline" : "online";
}
