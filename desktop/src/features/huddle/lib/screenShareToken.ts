import { getRelayHttpUrl, signRelayEvent } from "@/shared/api/tauri";

const NIP98_KIND = 27235;
const REQUEST_TIMEOUT_MS = 15_000;

export type ScreenShareIntent = "subscribe" | "publish";

export type ScreenTokenResponse = {
  url: string;
  token: string;
  room: string;
  can_publish: boolean;
  current_sharer: string | null;
  expires_in_secs?: number;
};

export type ScreenShareUnavailable = {
  unavailable: true;
  reason: string;
};

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
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

export function isScreenShareUnavailableError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const msg = error.message.toLowerCase();
  return (
    msg.includes("screen_share_unavailable") ||
    msg.includes("livekit is not configured")
  );
}

export function isScreenShareBusyError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  return error.message.toLowerCase().includes("screen_share_busy");
}

/**
 * Mint a short-lived LiveKit token for huddle screen share.
 * Returns `unavailable` when the relay has no LiveKit config (503).
 */
export async function mintScreenShareToken(args: {
  channelId: string;
  parentChannelId?: string | null;
  intent: ScreenShareIntent;
}): Promise<ScreenTokenResponse | ScreenShareUnavailable> {
  const base = (await getRelayHttpUrl()).replace(/\/+$/, "");
  const path = `/api/huddle/${args.channelId}/screen-token`;
  const url = `${base}${path}`;
  const body = JSON.stringify({
    parent_channel_id: args.parentChannelId ?? undefined,
    intent: args.intent,
  });
  const authorization = await nip98PostHeader(url, body);
  const response = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  const json = (await response.json().catch(() => ({}))) as Record<
    string,
    unknown
  >;
  if (response.status === 503 || json.error === "screen_share_unavailable") {
    return {
      unavailable: true,
      reason:
        typeof json.message === "string"
          ? json.message
          : "screen_share_unavailable",
    };
  }
  if (!response.ok) {
    const message =
      typeof json.error === "string"
        ? typeof json.message === "string"
          ? `${json.error}: ${json.message}`
          : json.error
        : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return json as ScreenTokenResponse;
}

/** Best-effort release of the relay soft sharer lock. */
export async function stopScreenShareSlot(args: {
  channelId: string;
  parentChannelId?: string | null;
}): Promise<void> {
  const base = (await getRelayHttpUrl()).replace(/\/+$/, "");
  const path = `/api/huddle/${args.channelId}/screen-stop`;
  const url = `${base}${path}`;
  const body = JSON.stringify({
    parent_channel_id: args.parentChannelId ?? undefined,
  });
  const authorization = await nip98PostHeader(url, body);
  await fetch(url, {
    method: "POST",
    headers: {
      Authorization: authorization,
      "Content-Type": "application/json",
    },
    body,
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  }).catch(() => {
    /* best-effort */
  });
}
