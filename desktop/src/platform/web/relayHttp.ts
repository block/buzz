// Browser port of desktop/src-tauri/src/relay.rs HTTP helpers: NIP-98 auth + POST /query, /events.
// Every request is signed through the pluggable Signer (NIP-07 extension) — the page never holds a key.
import { relayFetchUrl } from "./relayProxy";
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
    // Dev: same-origin /__relay proxy; production: the relay directly (or a CDN prefix). See relayProxy.ts. The NIP-98 `u`
  // tag above is still signed for the real relay URL.
  const fetchUrl = relayFetchUrl(relayHttpBase(), path);
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
/** Relay NIP-11 `max_filters` for a WebSocket REQ (crates/buzz-relay/src/protocol.rs MAX_FILTERS_PER_REQ). */
export const WS_MAX_FILTERS = 10;

export async function queryRelay(filters: Filter[]): Promise<NostrEvent[]> {
  if (canUseWs(filters)) {
    try {
      // The relay rejects a WebSocket REQ with more than 10 filters (NIP-11 max_filters) without a CLOSED for the sub,
      // which would stall until the query timeout. Split into parallel REQs on the same authenticated socket.
      if (filters.length > WS_MAX_FILTERS) {
        const chunks: Filter[][] = [];
        for (let i = 0; i < filters.length; i += WS_MAX_FILTERS)
          chunks.push(filters.slice(i, i + WS_MAX_FILTERS));
        return (await Promise.all(chunks.map((c) => wsQuery(c)))).flat();
      }
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

/** GET a relay JSON endpoint with NIP-98 (port of relay/get.rs get_relay_json), e.g. /workflows/<id>/runs. */
export async function getRelayJson<T = unknown>(pathWithQuery: string): Promise<T> {
  if (!pathWithQuery.startsWith("/")) throw new Error("relay GET path must begin with '/'");
  const url = `${relayHttpBase()}${pathWithQuery}`;
  const auth = await nip98Header("GET", url, null, 1);
  let res: Response;
  try {
    res = await fetch(relayFetchUrl(relayHttpBase(), pathWithQuery), { headers: { Authorization: auth, Accept: "application/json" } });
  } catch (e) {
    throw new RelayHttpError(`relay unreachable: could not connect to relay (${e instanceof Error ? e.message : e})`);
  }
  if (!res.ok) throw new RelayHttpError(await relayError(res));
  return (await res.json()) as T;
}
