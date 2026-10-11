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
export const D_TX_PREFIX = `${D_PREFIX}tx/`;
export const D_CMD_PREFIX = `${D_PREFIX}cmd/`;
export const D_RES_PREFIX = `${D_PREFIX}res/`;
export const txChunkD = (agentPubkey: string, seq: number) =>
  `${D_TX_PREFIX}${agentPubkey.toLowerCase()}/${seq}`;
export const cmdD = (cmdId: string) => `${D_CMD_PREFIX}${cmdId}`;
export const resD = (cmdId: string) => `${D_RES_PREFIX}${cmdId}`;

/** Transcript chunk plaintext cap (bytes of JSON body). */
export const MAX_CHUNK_BYTES = 16 * 1024;
/** Per-entry text cap after redaction. */
export const MAX_ENTRY_TEXT = 1500;
/** On first publish for an agent, only this many most-recent entries are sent. */
export const TX_INITIAL_BACKLOG = 40;
/** Chunk events expire after 6 h (relay may ignore; clients enforce). */
export const TX_TTL_S = 6 * 3600;
export const TX_DEBOUNCE_MS = 1_500;
export const TX_MIN_INTERVAL_MS = 4_000;
/** Global cap on transcript chunk events per rolling minute (protects the relay). */
export const TX_MAX_EVENTS_PER_MIN = 24;
/** Web keeps at most this many entries per agent. */
export const WEB_MAX_ENTRIES_PER_AGENT = 300;

export const CMD_MAX_TTL_S = 120;
export const CMD_MAX_TEXT = 4000;
export const CMD_RESULT_TTL_S = 24 * 3600;
export const CMD_SEEN_LIMIT = 1000;

export type CommandType =
  | "agent.start"
  | "agent.stop"
  | "agent.message"
  | "prompt.approve"
  | "prompt.deny"
  | "browser.open"
  | "browser.close"
  | "browser.screenshot";
/** Commands that change state or send content: the web must get an explicit confirmation before sending. */
export const DESTRUCTIVE_COMMANDS: readonly CommandType[] = [
  "agent.stop",
  "agent.message",
  "prompt.approve",
  "prompt.deny",
  "browser.open",
  "browser.close",
];
/** browser.screenshot is view-only and user-initiated per press, so it needs no confirm; the desktop still rate-limits it. */
export const ALL_COMMANDS: readonly CommandType[] = [
  "agent.start",
  ...DESTRUCTIVE_COMMANDS,
  "browser.screenshot",
];

// ---- Slice R: desktop browser sessions (view-only) ----
export const D_SHOT_PREFIX = `${D_PREFIX}shot/`;
export const D_BROWSER_SESSIONS = `${D_PREFIX}snap/browser`;
export const shotD = (cmdId: string, i: number) => `${D_SHOT_PREFIX}${cmdId}/${i}`;
/** Image bytes per chunk event (keeps the encrypted event content well under the relay's ~64 KiB content limit). */
export const SHOT_CHUNK_BYTES = 24 * 1024;
export const SHOT_MAX_CHUNKS = 5;
export const SHOT_MAX_BYTES = SHOT_CHUNK_BYTES * SHOT_MAX_CHUNKS;
export const SHOT_MAX_WIDTH = 1000;
export const SHOT_TTL_S = 300;
/** At most one screenshot per session every 3 s, 10 per minute overall, one in flight. */
export const SHOT_MIN_INTERVAL_MS = 3_000;
export const SHOT_MAX_PER_MIN = 10;
export const BROWSER_MAX_SESSIONS = 2;
export const BROWSER_MAX_URL = 2048;

export type BrowserSessionInfo = {
  sessionId: string;
  title: string;
  host: string;
  openedAt: number;
};
export type BrowserSessionsBody = { sessions: BrowserSessionInfo[] };
export type ShotChunkBody = {
  sessionId: string;
  mime: string;
  w: number;
  h: number;
  i: number;
  n: number;
  /** base64 of this chunk's image bytes */
  data: string;
};

export type CommandBody =
  | { agentPubkey: string }
  | { agentPubkey: string; channelId: string; text: string }
  | { agentPubkey: string; promptId: string }
  | { url: string }
  | { sessionId: string };

export type CommandResultBody = {
  status: "accepted" | "ok" | "error";
  code?: string;
  message?: string;
  /** Small structured result (e.g. {sessionId} for browser.open, {chunks} for browser.screenshot). */
  result?: Record<string, unknown>;
};

export type TranscriptRole =
  | "assistant"
  | "user"
  | "tool"
  | "thought"
  | "plan"
  | "status"
  | "permission"
  | "error";

export type TranscriptEntry = {
  /** Stable id of the transcript item (same id + later content = update). */
  id: string;
  /** Item timestamp, unix ms. */
  t: number;
  role: TranscriptRole;
  /** Redacted, clipped text. */
  text: string;
  /** For tool entries. */
  tool?: { name: string; status: string; error?: boolean };
  channelId?: string;
};

export type TranscriptChunkBody = {
  agentPubkey: string;
  entries: TranscriptEntry[];
};

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

export type BridgeType =
  | "agents.snapshot"
  | "presence.heartbeat"
  | "transcript.chunk"
  | "browser.sessions"
  | "browser.shot"
  | "cmd.result"
  | CommandType;

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
