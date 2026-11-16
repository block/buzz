import {
  type GiphyGif,
  type GiphyResponse,
  normalizeGiphyGifs,
  relayGiphyCapability,
  type RelayGiphyCapability,
  type RelayGifSearchInfo,
} from "@/features/gifs/api";
import { relayHttpFromWs } from "@/shared/api/inviteHelpers";
import { signRelayEvent } from "@/shared/api/tauri";

const GIPHY_CUSTOMER_ID_STORAGE_KEY_PREFIX = "buzz:giphy-customer-id:v1:";
const NIP98_KIND = 27235;

function customerId(relayUrl: string): string {
  if (typeof window === "undefined") return globalThis.crypto.randomUUID();

  try {
    const storageKey = `${GIPHY_CUSTOMER_ID_STORAGE_KEY_PREFIX}${relayUrl}`;
    const existing = window.localStorage.getItem(storageKey);
    if (existing) return existing;

    const created = globalThis.crypto.randomUUID();
    window.localStorage.setItem(storageKey, created);
    return created;
  } catch {
    // Storage can be unavailable in hardened webviews. Prefer an ephemeral ID
    // over a process-wide fallback that would correlate unrelated relays.
    return globalThis.crypto.randomUUID();
  }
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

async function nip98PostHeader(url: string, body: string): Promise<string> {
  const authEvent = await signRelayEvent({
    kind: NIP98_KIND,
    content: "",
    tags: [
      ["u", url],
      ["method", "POST"],
      ["payload", await sha256Hex(body)],
      ["nonce", crypto.randomUUID()],
    ],
  });
  return `Nostr ${btoa(JSON.stringify(authEvent))}`;
}

const FRIENDLY_GIF_ERRORS: Record<string, string> = {
  relay_membership_required: "Join this community to search GIFs.",
};

function gifErrorMessage(error: string | undefined, status: number): string {
  if (error && FRIENDLY_GIF_ERRORS[error]) return FRIENDLY_GIF_ERRORS[error];
  return error || `GIF request failed (${status})`;
}

async function relayPost<T>(
  relayUrl: string,
  path: string,
  payload: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<T> {
  const url = `${relayHttpFromWs(relayUrl).replace(/\/+$/, "")}${path}`;
  const body = JSON.stringify(payload);
  const response = await fetch(url, {
    body,
    headers: {
      Authorization: await nip98PostHeader(url, body),
      "Content-Type": "application/json",
    },
    method: "POST",
    signal,
  });
  if (!response.ok) {
    const json = (await response.json().catch(() => ({}))) as {
      error?: string;
    };
    throw new Error(gifErrorMessage(json.error, response.status));
  }
  return (await response.json()) as T;
}

/** The selected relay's advertised GIPHY endpoints, when supported. */
export async function relayGiphyEndpoints(
  relayUrl: string,
  signal?: AbortSignal,
): Promise<RelayGiphyCapability | null> {
  const url = `${relayHttpFromWs(relayUrl).replace(/\/+$/, "")}/info`;
  const response = await fetch(url, {
    headers: { Accept: "application/nostr+json" },
    signal,
  });
  if (!response.ok)
    throw new Error(`Could not read relay capabilities (${response.status})`);
  const info = (await response.json()) as RelayGifSearchInfo;
  return relayGiphyCapability(info);
}

/** Search GIPHY through the selected relay without exposing its provider key. */
export async function fetchGiphyGifs(
  relayUrl: string,
  searchPath: string,
  query: string,
  signal?: AbortSignal,
): Promise<GiphyGif[]> {
  const response = await relayPost<GiphyResponse>(
    relayUrl,
    searchPath,
    {
      customer_id: customerId(relayUrl),
      locale: navigator.language || "en-US",
      query: query.trim(),
    },
    signal,
  );
  return normalizeGiphyGifs(Array.isArray(response.data) ? response.data : []);
}
