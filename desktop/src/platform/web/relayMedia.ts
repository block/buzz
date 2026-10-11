// Web media: auth-gated relay media (GET /media/<sha>.<ext>) and uploads (PUT /upload) for the browser build.
//
// Desktop loads relay media through a localhost proxy that adds a Blossom `t=get` Authorization header (kind 24242). A plain
// <img src> cannot send that header, and the relay's CORS answer does not allow our origin, so the web build fetch()es the bytes
// through the same-origin /__relay proxy with the header and shows them as blob: URLs. One signed token is reused for every
// media request while it is valid (Blossom tokens are expiry-bound, not single-use), so a slow remote signer is asked once
// per ~40 s instead of once per image.
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { relayHttpBase } from "./relayConfig";
import { getSigner } from "./signer";

const TOKEN_TTL_SECS = 60; // relay freshness window (NIP-FI)
const TOKEN_REUSE_SECS = 40; // reuse a token only for this long after minting
const MAX_CONCURRENT_FETCHES = 4;
const MAX_CACHED_BLOBS = 300;

const RELAY_MEDIA_RE =
  /^https?:\/\/[^/]+\/media\/([\da-f]{64}(?:\.thumb)?\.[a-z0-9]{2,5})(?:\?.*)?$/i;

const b64url = (s: string) => {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
};

function serverAuthority(base: string): string {
  const u = new URL(base);
  return u.host;
}

/** Same-origin proxy URL for a relay path (the relay does not send CORS headers for our origin). */
export function proxiedRelayUrl(base: string, path: string): string {
  return new URL(base).origin === location.origin
    ? `${base}${path}`
    : `/__relay/${encodeURIComponent(base)}${path}`;
}

async function signBlossom(
  t: "get" | "upload",
  base: string,
  extraTags: string[][],
  priority: 0 | 2,
  nowSecs: number,
): Promise<string> {
  const ev = await getSigner().signEvent(
    {
      kind: 24242,
      content: t === "get" ? "Get buzz-media" : "Upload buzz-media",
      created_at: nowSecs,
      tags: [
        ["t", t],
        ...extraTags,
        ["expiration", String(nowSecs + TOKEN_TTL_SECS)],
        ["server", serverAuthority(base)],
      ],
    },
    { priority },
  );
  return `Nostr ${b64url(JSON.stringify(ev))}`;
}

// ── GET token (shared) ───────────────────────────────────────────────────────
let tokenCache: { base: string; header: string; mintedAt: number } | null = null;
let tokenInflight: Promise<string> | null = null;
export const signerRoundTrips = { get: 0 };

export function resetMediaTokenCache() {
  tokenCache = null;
  tokenInflight = null;
}

export async function mediaGetAuth(force = false): Promise<string> {
  const base = relayHttpBase();
  const now = Math.floor(Date.now() / 1000);
  if (
    !force &&
    tokenCache &&
    tokenCache.base === base &&
    now - tokenCache.mintedAt < TOKEN_REUSE_SECS
  )
    return tokenCache.header;
  if (tokenInflight) return tokenInflight;
  signerRoundTrips.get += 1;
  const p = signBlossom("get", base, [], 2, now)
    .then((header) => {
      tokenCache = { base, header, mintedAt: now };
      return header;
    })
    .finally(() => {
      if (tokenInflight === p) tokenInflight = null;
    });
  tokenInflight = p;
  return p;
}

// ── fetch queue + blob cache ─────────────────────────────────────────────────
export class MediaFetchError extends Error {
  constructor(
    message: string,
    public status?: number,
  ) {
    super(message);
    this.name = "MediaFetchError";
  }
}

/** True when `url` is media on the active relay that must be fetched with auth. */
export function isRelayMediaUrl(url: string | undefined | null): boolean {
  if (!url || !RELAY_MEDIA_RE.test(url)) return false;
  try {
    return new URL(url).origin === new URL(relayHttpBase()).origin;
  } catch {
    return false;
  }
}

const blobCache = new Map<string, string>(); // insertion order = LRU
const inflight = new Map<string, Promise<string>>();
let active = 0;
const waiting: Array<() => void> = [];

async function slot<T>(run: () => Promise<T>): Promise<T> {
  if (active >= MAX_CONCURRENT_FETCHES)
    await new Promise<void>((r) => waiting.push(r));
  active += 1;
  try {
    return await run();
  } finally {
    active -= 1;
    waiting.shift()?.();
  }
}

async function fetchOnce(url: string, auth: string): Promise<Response> {
  const path = new URL(url).pathname;
  return fetch(proxiedRelayUrl(new URL(url).origin, path), {
    headers: { Authorization: auth },
  });
}

async function loadBlob(url: string): Promise<string> {
  return slot(async () => {
    let res = await fetchOnce(url, await mediaGetAuth());
    if (res.status === 401 || res.status === 403)
      res = await fetchOnce(url, await mediaGetAuth(true));
    if (!res.ok)
      throw new MediaFetchError(
        res.status === 404
          ? "Image not found on the relay"
          : `Relay returned ${res.status}`,
        res.status,
      );
    const blob = await res.blob();
    return URL.createObjectURL(blob);
  });
}

/** Cached blob: URL for an auth-gated relay media URL (deduplicated, bounded concurrency). */
export function getRelayMediaBlobUrl(url: string): Promise<string> {
  const hit = blobCache.get(url);
  if (hit) {
    blobCache.delete(url);
    blobCache.set(url, hit);
    return Promise.resolve(hit);
  }
  const running = inflight.get(url);
  if (running) return running;
  const p = loadBlob(url)
    .then((blobUrl) => {
      blobCache.set(url, blobUrl);
      while (blobCache.size > MAX_CACHED_BLOBS) {
        const oldest = blobCache.keys().next().value as string;
        const old = blobCache.get(oldest);
        blobCache.delete(oldest);
        if (old) setTimeout(() => URL.revokeObjectURL(old), 60_000);
      }
      return blobUrl;
    })
    .finally(() => inflight.delete(url));
  inflight.set(url, p);
  return p;
}

export function peekRelayMediaBlobUrl(url: string): string | undefined {
  return blobCache.get(url);
}

// ── upload ───────────────────────────────────────────────────────────────────
export type BlobDescriptor = {
  url: string;
  sha256: string;
  size: number;
  type: string;
  uploaded: number;
  filename?: string;
  [k: string]: unknown;
};

export function sniffMime(b: Uint8Array, fallback = ""): string {
  const s = (i: number, str: string) =>
    [...str].every((c, k) => b[i + k] === c.charCodeAt(0));
  if (b[0] === 0x89 && s(1, "PNG")) return "image/png";
  if (b[0] === 0xff && b[1] === 0xd8) return "image/jpeg";
  if (s(0, "GIF8")) return "image/gif";
  if (s(0, "RIFF") && s(8, "WEBP")) return "image/webp";
  if (s(4, "ftyp")) return "video/mp4";
  if (b[0] === 0x1a && b[1] === 0x45 && b[2] === 0xdf && b[3] === 0xa3)
    return "video/webm";
  return fallback || "application/octet-stream";
}

export async function uploadRelayMedia(
  bytes: Uint8Array,
  opts: { filename?: string; mime?: string } = {},
): Promise<BlobDescriptor> {
  const base = relayHttpBase();
  const hash = bytesToHex(sha256(bytes));
  const mime = sniffMime(bytes, opts.mime);
  const now = Math.floor(Date.now() / 1000);
  const auth = await signBlossom("upload", base, [["x", hash]], 0, now);
  const send = (path: string) =>
    fetch(proxiedRelayUrl(base, path), {
      method: "PUT",
      headers: {
        Authorization: auth,
        "Content-Type": mime,
        "X-SHA-256": hash,
      },
      body: bytes as unknown as BodyInit,
    });
  let res = await send("/upload");
  if (res.status === 404 || res.status === 405) res = await send("/media/upload");
  if (!res.ok) {
    let msg = `Upload failed (${res.status})`;
    try {
      const j = await res.json();
      if (j?.error) msg = `Upload failed: ${j.error}`;
    } catch {
      /* not json */
    }
    throw new Error(msg);
  }
  const d = (await res.json()) as BlobDescriptor;
  if (opts.filename) d.filename = opts.filename.replace(/[\\/]/g, "_").slice(0, 200);
  return d;
}

/** Hidden <input type=file> picker for the attach button (web has no native dialog). */
export function pickFilesFromBrowser(accept?: string): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = true;
    if (accept) input.accept = accept;
    input.style.display = "none";
    const done = (files: File[]) => {
      input.remove();
      resolve(files);
    };
    input.addEventListener("change", () => done([...(input.files ?? [])]));
    input.addEventListener("cancel", () => done([]));
    document.body.appendChild(input);
    input.click();
  });
}

