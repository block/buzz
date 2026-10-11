// Slice R policy (pure): URL validation, remote-session limit, per-session and global screenshot rate limits, one in flight.
import {
  BROWSER_MAX_SESSIONS,
  BROWSER_MAX_URL,
  SHOT_CHUNK_BYTES,
  SHOT_MAX_BYTES,
  SHOT_MAX_WIDTH,
  SHOT_MAX_PER_MIN,
  SHOT_MIN_INTERVAL_MS,
  type BrowserSessionInfo,
} from "./protocol";

const PRIVATE_HOST =
  /^(localhost|.*\.localhost|.*\.local|.*\.internal|.*\.lan|0\.0\.0\.0|\[?::1?\]?|127\.\d+\.\d+\.\d+|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+|172\.(1[6-9]|2\d|3[01])\.\d+\.\d+|169\.254\.\d+\.\d+|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.\d+\.\d+)$/i;

export type UrlCheck = { ok: true; url: string; host: string } | { ok: false; code: string; message: string };

/** http(s) only, no credentials, no private/loopback/tailnet targets, bounded length. */
export function validateBrowserUrl(raw: unknown): UrlCheck {
  if (typeof raw !== "string" || !raw.trim() || raw.length > BROWSER_MAX_URL)
    return { ok: false, code: "bad_url", message: "Invalid URL." };
  let u: URL;
  try {
    u = new URL(raw.trim());
  } catch {
    return { ok: false, code: "bad_url", message: "Invalid URL." };
  }
  if (u.protocol !== "https:" && u.protocol !== "http:")
    return { ok: false, code: "bad_url", message: "Only http and https links can be opened." };
  if (u.username || u.password)
    return { ok: false, code: "bad_url", message: "Links with embedded credentials are not allowed." };
  if (PRIVATE_HOST.test(u.hostname))
    return { ok: false, code: "blocked_url", message: "Private and local addresses can't be opened remotely." };
  return { ok: true, url: u.toString(), host: u.hostname };
}

export class BrowserSessionManager {
  private sessions = new Map<string, BrowserSessionInfo>();
  private lastShotAt = new Map<string, number>();
  private shotTimes: number[] = [];
  private inflight = false;
  private listeners = new Set<() => void>();
  private max: number;
  constructor(max = BROWSER_MAX_SESSIONS) {
    this.max = max;
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
  list(): BrowserSessionInfo[] {
    return [...this.sessions.values()];
  }
  has(id: string) {
    return this.sessions.has(id);
  }
  get(id: string) {
    return this.sessions.get(id);
  }
  canOpen() {
    return this.sessions.size < this.max;
  }
  add(info: BrowserSessionInfo) {
    this.sessions.set(info.sessionId, info);
    this.emit();
  }
  remove(id: string) {
    this.sessions.delete(id);
    this.lastShotAt.delete(id);
    this.emit();
  }
  /** Returns null if a screenshot may start now, otherwise a reason code. Marks it in flight on success. */
  beginShot(id: string, nowMs: number): null | "busy" | "rate_limited" {
    if (this.inflight) return "busy";
    this.shotTimes = this.shotTimes.filter((t) => nowMs - t < 60_000);
    const last = this.lastShotAt.get(id) ?? 0;
    if (nowMs - last < SHOT_MIN_INTERVAL_MS || this.shotTimes.length >= SHOT_MAX_PER_MIN)
      return "rate_limited";
    this.inflight = true;
    this.lastShotAt.set(id, nowMs);
    this.shotTimes.push(nowMs);
    return null;
  }
  endShot() {
    this.inflight = false;
  }
}

/** Splits image bytes into publishable chunks; null if over the size cap. */
export function chunkImage(bytes: Uint8Array): Uint8Array[] | null {
  if (bytes.length === 0 || bytes.length > SHOT_MAX_BYTES) return null;
  const out: Uint8Array[] = [];
  for (let o = 0; o < bytes.length; o += SHOT_CHUNK_BYTES) out.push(bytes.subarray(o, o + SHOT_CHUNK_BYTES));
  return out;
}

export const bytesToBase64 = (b: Uint8Array) => {
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
};
export const base64ToBytes = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

export type Encoded = { bytes: Uint8Array; w: number; h: number };

/** Shrinks until the image fits SHOT_MAX_BYTES: first lower quality, then lower width. Null if it never fits. */
export async function fitImage(
  encode: (maxW: number, quality: number) => Promise<Encoded>,
  startW = SHOT_MAX_WIDTH,
): Promise<Encoded | null> {
  for (let w = startW; w >= 320; w = Math.floor(w * 0.75)) {
    for (const q of [0.7, 0.5, 0.35]) {
      const e = await encode(w, q);
      if (e.bytes.length <= SHOT_MAX_BYTES) return e;
    }
  }
  return null;
}

