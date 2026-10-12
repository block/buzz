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
  | "browser.screenshot"
  | "browser.agent"
  | "term.handoff";
/** Commands that change state or send content: the web must get an explicit confirmation before sending. */
export const DESTRUCTIVE_COMMANDS: readonly CommandType[] = [
  "agent.stop",
  "agent.message",
  "prompt.approve",
  "prompt.deny",
  "browser.open",
  "browser.close",
  "browser.agent",
  "term.handoff",
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

export type BrowserAgentMode = "observe" | "drive";
/** One browser of the desktop's Browsers page, as published to the member's own devices (no secrets: query/hash stripped, only allowed URLs). */
export type BrowserSessionInfo = {
  sessionId: string;
  title: string;
  host: string;
  openedAt: number;
  /** Allowed page URL without query or fragment. Absent when the page is a private/local address (listed as restricted). */
  url?: string;
  /** "cold" = the desktop webview isn't running yet (a screenshot will start it). */
  status?: "live" | "cold";
  /** Where it was added: from a web/mobile device through the bridge, or on the desktop itself. */
  origin?: "web" | "desktop";
  /** True when the URL is private/local/tailnet and so isn't shared. */
  restricted?: boolean;
  /** The agent currently observing/driving it, if any. */
  agent?: { pubkey: string; name: string; mode: BrowserAgentMode };
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

// ---- term.handoff: "Pass thread context to Buzz Term" from the web ----
export type TermHandoffHarness = "claude" | "codex";
export const TERM_HANDOFF_HARNESSES: readonly TermHandoffHarness[] = ["claude", "codex"];
export type TermHandoffBody = {
  agentPubkey: string;
  channelId: string;
  /** Event id of the thread head. */
  threadId: string;
  harness: TermHandoffHarness;
};
/** How long the desktop waits for the chosen agent's handoff card (must stay under CMD_MAX_TTL_S so the web does not time out first). */
export const TERM_HANDOFF_WAIT_MS = 100_000;
/** Interactive harnesses installed on the desktop, advertised in the heartbeat. No paths: just availability. */
export type HarnessInfo = { id: TermHandoffHarness; available: boolean };
/** Cap on channel ids advertised per agent (running agents only). */
export const MAX_AGENT_CHANNEL_IDS = 60;

export type CommandBody =
  | { agentPubkey: string }
  | { agentPubkey: string; channelId: string; text: string }
  | { agentPubkey: string; promptId: string }
  | { url: string }
  | { sessionId: string }
  | { sessionId: string; mode: "off" | BrowserAgentMode; agentPubkey?: string; channelId?: string }
  | TermHandoffBody;

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
  /** Channels this agent is a member of (running/deployed agents only, capped). Used by the web "Pass thread context to Buzz Term" picker. */
  channelIds?: string[];
};

export type AgentsSnapshotBody = {
  agents: AgentSummary[];
  truncated?: boolean;
  generatedAt: number;
};

/** Slice B: what a desktop advertises about its direct (tailnet) listener, inside the encrypted heartbeat. */
export type RemoteInfo = {
  desktopId: string;
  name: string;
  /** wss://<tailnet-host>:9443/v1/ws (ws://127.0.0.1 only for local dev/e2e) */
  url: string;
  status: "on" | "off";
  /** Why it's off (e.g. "Tailscale isn't running on this Mac."). */
  reason?: string;
  /** Second, separate toggle: "Allow remote terminal" (tailnet only). */
  terminals: boolean;
  /** What a new remote terminal runs on this desktop, from Settings → Buzz Term → Session host ("herdr" = attach/create a herdr session). */
  termHost?: "shell" | "herdr";
  /** Default herdr session name from Settings → Buzz Term (only meaningful when termHost is "herdr"; blank = herdr's unnamed default). */
  termSession?: string;
  /** "Allow remote agent management" is on (list/start/stop/message over relay; full management on a direct connection). */
  agents?: boolean;
  /** How long one signed connection challenge stays valid on this desktop (seconds). */
  sigTtlS?: number;
};

/** Connection-signature lifetimes offered on the desktop (max 24 h, default 1 h). */
export const SIG_TTL_OPTIONS_S = [15 * 60, 60 * 60, 4 * 3600, 12 * 3600, 24 * 3600] as const;
export const SIG_TTL_DEFAULT_S = 3600;
export const SIG_TTL_MAX_S = 24 * 3600;
/** A host not heard from for this long is shown greyed out (not hidden). */
export const HOST_STALE_S = 24 * 3600;

export type HeartbeatBody = {
  remote?: RemoteInfo;
  bridge: "v1";
  appVersion?: string;
  platform?: string;
  agentsRunning: number;
  uptimeSec: number;
  /** Interactive Buzz Term harnesses (claude/codex) found on this desktop. */
  harnesses?: HarnessInfo[];
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
  private last = new Map<string, { seq: number; ts: number }>();
  accept(env: Envelope): boolean {
    const k = `${env.dev}|${env.kid}|${env.type}`;
    const prev = this.last.get(k);
    // A desktop window reload restarts the publisher's counter under the same key: a strictly newer envelope time is a
    // restart, not a replay, so accept it (otherwise heartbeats were dropped for minutes until the counter caught up).
    if (prev !== undefined && (env.ts < prev.ts || (env.seq <= prev.seq && !(env.ts > prev.ts)))) return false;
    this.last.set(k, { seq: env.seq, ts: env.ts });
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
