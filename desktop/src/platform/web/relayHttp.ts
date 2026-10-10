// Browser port of desktop/src-tauri/src/relay.rs HTTP helpers: NIP-98 auth + POST /query, /events.
// Every request is signed through the pluggable Signer (NIP-07 extension) — the page never holds a key.
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { getSigner } from "./signer";
import type { SignPriority } from "./signer/types";
import type { Filter, NostrEvent } from "./nostrTypes";
import { relayHttpBase } from "./relayConfig";
import { canUseWs, wsQuery } from "./relayWsQuery";

const utf8 = (s: string) => new TextEncoder().encode(s);
const b64 = (s: string) => {
  const bytes = utf8(s);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
};

/** NIP-98 (kind 27235) Authorization header value: `Nostr <base64(event)>`. */
export async function nip98Header(
  method: string,
  url: string,
  body: Uint8Array | null,
  priority?: SignPriority,
): Promise<string> {
  const payload = bytesToHex(sha256(body ?? new Uint8Array()));
  const ev = await getSigner().signEvent(
    {
      kind: 27235,
      content: "",
      tags: [
        ["u", url],
        ["method", method],
        ["payload", payload],
        ["nonce", crypto.randomUUID()],
      ],
    },
    { priority },
  );
  return `Nostr ${b64(JSON.stringify(ev))}`;
}

export class RelayHttpError extends Error {}

async function relayError(res: Response): Promise<string> {
  const text = await res.text().catch(() => "");
  try {
    const j = JSON.parse(text);
    if (j?.error) return `relay returned ${res.status}: ${j.error}`;
  } catch {
    /* not json */
  }
  return `relay returned ${res.status}`;
}

// Identical concurrent read POSTs (/query only) share ONE NIP-98 signature and ONE request. We deliberately do NOT reuse a
// signed token across later requests: the relay marks each NIP-98 event id as seen (replay guard), so reuse would be rejected.
const inflightQueries = new Map<string, Promise<unknown>>();

// Signature priority for the NIP-98 token: writes (POST /events) are user actions (P0); reads are P1; a presence poll is background (P3,
// droppable: a slow signer must never queue it ahead of a send or a channel read).
const isPresenceRead = (body: unknown) =>
  Array.isArray(body) && body.length > 0 && body.every((f) => Array.isArray(f?.kinds) && f.kinds.every((k: number) => k === 20001 || k === 20002));

async function post(path: string, body: unknown): Promise<unknown> {
  if (path !== "/query") return postOnce(path, body);
  const key = `${relayHttpBase()}${path}\0${JSON.stringify(body)}`;
  const hit = inflightQueries.get(key);
  if (hit) return hit;
  const p = postOnce(path, body);
  inflightQueries.set(key, p);
  const clear = () => void inflightQueries.delete(key);
  p.then(clear, clear);
  return p;
}

async function postOnce(path: string, body: unknown): Promise<unknown> {
  const url = `${relayHttpBase()}${path}`;
  const bytes = utf8(JSON.stringify(body));
  const prio: SignPriority = path === "/events" ? 0 : isPresenceRead(body) ? 3 : 1;
  const auth = await nip98Header("POST", url, bytes, prio);
  let res: Response;
  // DEV only: a cross-origin relay (e.g. prod from http://127.0.0.1:5199) fails CORS on /query and /events, so go through the
  // Vite dev proxy (same-origin). The NIP-98 `u` tag above is still signed for the real relay URL.
  const fetchUrl =
    import.meta.env.DEV && new URL(url).origin !== location.origin
      ? `/__relay/${encodeURIComponent(relayHttpBase())}${path}`
      : url;
  try {
    res = await fetch(fetchUrl, {
      method: "POST",
      headers: { Authorization: auth, "Content-Type": "application/json" },
      body: bytes,
    });
  } catch (e) {
    console.error(
      `[relay-http] POST ${url} failed before a response (network error or CORS block from ${location.origin}):`,
      e,
    );
    throw new RelayHttpError(
      `relay unreachable: could not connect to relay (${e instanceof Error ? e.message : e}; possible CORS block for ${location.origin})`,
    );
  }
  if (!res.ok) {
    const msg = await relayError(res);
    console.error(`[relay-http] POST ${url} -> ${msg}`);
    throw new RelayHttpError(msg);
  }
  return res.json();
}

/** POST /query — one-shot filter query over HTTP (NIP-98 per request). */
export async function queryRelayHttp(filters: Filter[]): Promise<NostrEvent[]> {
  return (await post("/query", filters)) as NostrEvent[];
}

/**
 * One-shot query. Prefers the authenticated WebSocket (one signature per session); falls back to HTTP /query for
 * bridge-only filters, ephemeral kinds, or if the socket path fails.
 */
export async function queryRelay(filters: Filter[]): Promise<NostrEvent[]> {
  if (canUseWs(filters)) {
    try {
      return await wsQuery(filters);
    } catch (e) {
      if (import.meta.env.DEV)
        console.warn(
          "[ws-query] falling back to HTTP:",
          e instanceof Error ? e.message : e,
        );
    }
  }
  return queryRelayHttp(filters);
}

export type SubmitResult = {
  event_id: string;
  accepted: boolean;
  message: string;
};
/** POST /events — submit an already signed event. */
export async function submitSignedEvent(
  event: NostrEvent,
): Promise<SubmitResult> {
  const r = (await post("/events", event)) as SubmitResult;
  if (!r.accepted)
    throw new RelayHttpError(`relay rejected event: ${r.message}`);
  return r;
}
