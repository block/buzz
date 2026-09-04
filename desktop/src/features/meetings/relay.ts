/**
 * Meetings control-plane client — talks to HiveTalk *through* the buzz-relay
 * proxy (`crates/buzz-relay/src/api/meetings.rs`).
 *
 * Every request carries **two independent signatures**:
 *
 *   1. the **buzz-relay NIP-98** in the standard `Authorization` header — proves
 *      the caller is a member of this community. Its `u` tag is the *relay*
 *      URL (`<relayHttp>/meetings/...`, query string included for signed GETs).
 *
 *   2. the caller's **own HiveTalk signature** — proves HiveTalk entitlement.
 *      For the challenge flow it rides in `X-Hivetalk-Authorization` /
 *      `X-Hivetalk-Challenge`; for `get-token` it is embedded in the JSON body;
 *      for moderation it is a LiveKit `Bearer <jwt>`. Its `u` tag is the
 *      *upstream* HiveTalk URL (`<apiBase>/api/...`) — that is what HiveTalk
 *      verifies against.
 *
 * Media (audio/video) never transits this module: the LiveKit SDK connects to
 * the SFU URL from `getMeetingToken` directly.
 */

import {
  classifyMeetingError,
  type ActiveRoom,
  type MeetingError,
  type MeetingPlan,
  type MeetingToken,
  type PaymentStatus,
  type RegisteredRoom,
  type RelayMeetingsCapability,
  type RelayMeetingsInfo,
  type SubscribeIntent,
  type SubscriptionStatus,
  normalizePlans,
  normalizeRooms,
  relayMeetingsCapability,
} from "@/features/meetings/api";
import { relayHttpFromWs } from "@/shared/api/inviteHelpers";
import { signRelayEvent } from "@/shared/api/tauri";

const NIP98_KIND = 27235;

/**
 * Relay-relative path prefix for the Meetings proxy. The relay registers these
 * routes at fixed paths (`crates/buzz-relay/src/router.rs`); the NIP-11
 * `proxy` field echoes this value for feature-detection but the routes are not
 * dynamic, so a constant is correct here.
 */
const MEETINGS_PREFIX = "/meetings";

/** HiveTalk challenge-flow action verbs (kind-27235 `action` tag). */
type HivetalkAction =
  | "subscribe"
  | "payment-status"
  | "create-room"
  | "edit-room"
  | "subscription";

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function relayHttpBase(relayWsUrl: string): string {
  return relayHttpFromWs(relayWsUrl).replace(/\/+$/, "");
}

// --- buzz-relay NIP-98 ---

/**
 * Build the buzz-relay `Authorization: Nostr <base64>` header.
 *
 * POST carries a `payload` tag (`sha256(body)`) — the relay's `authenticate`
 * passes `require_payload = true` for POST routes. GET carries no payload tag
 * (`require_payload = false`), but `url` **must** already include the query
 * string: the relay reconstructs `expected_url` as `path?rawquery` and
 * `verify_nip98_event` compares it to the `u` tag.
 */
async function buzzNip98(
  url: string,
  method: "GET" | "POST",
  body?: string,
): Promise<string> {
  const tags: string[][] = [
    ["u", url],
    ["method", method],
    ["nonce", crypto.randomUUID()],
  ];
  if (method === "POST") {
    tags.push(["payload", await sha256Hex(body ?? "")]);
  }
  const event = await signRelayEvent({ kind: NIP98_KIND, content: "", tags });
  return `Nostr ${btoa(JSON.stringify(event))}`;
}

// --- HiveTalk signatures ---

async function hivetalkChallenge(
  relayWsUrl: string,
  signal?: AbortSignal,
): Promise<{
  challenge: string;
  nonce: string;
  expires_at: string;
  domain: string;
}> {
  return meetingsRequest(relayWsUrl, "GET", "/auth/challenge", {
    signal,
  });
}

/**
 * Sign the caller's own HiveTalk challenge-flow request (Pattern 1). Returns
 * the `X-Hivetalk-Authorization` header value.
 *
 * The `u` tag is the **upstream HiveTalk URL**, not the relay URL — HiveTalk
 * verifies this signature and expects to see its own address.
 */
async function signHivetalkAction(params: {
  action: HivetalkAction;
  apiBase: string;
  upstreamPath: string;
  method: "GET" | "POST";
  rawBody: string;
  nonce: string;
  query?: string;
}): Promise<string> {
  const { action, apiBase, upstreamPath, method, rawBody, nonce, query } =
    params;
  const u = `${apiBase}${upstreamPath}${query ? `?${query}` : ""}`;
  const event = await signRelayEvent({
    kind: NIP98_KIND,
    content: "",
    tags: [
      ["payload", await sha256Hex(rawBody)],
      ["action", action],
      ["nonce", nonce],
      ["u", u],
      ["method", method],
    ],
  });
  return `Nostr ${btoa(JSON.stringify(event))}`;
}

/**
 * Build the `get-token` request body (HiveTalk Pattern 2). The signed event
 * rides inside `attributes.signed_event` as a JSON string; HiveTalk requires
 * `created_at` within ±5 minutes and `pubkey` to equal the event pubkey.
 */
async function buildGetTokenBody(
  apiBase: string,
  roomName: string,
  participantName: string,
  pubkeyHex: string,
): Promise<string> {
  const signedEvent = await signRelayEvent({
    kind: NIP98_KIND,
    content: "",
    createdAt: Math.floor(Date.now() / 1000),
    tags: [
      ["u", `${apiBase}/api/get-token`],
      ["method", "POST"],
    ],
  });
  return JSON.stringify({
    roomName,
    participantName,
    pubkey: pubkeyHex,
    attributes: { signed_event: JSON.stringify(signedEvent) },
  });
}

// --- transport ---

type HivetalkHeaders = {
  authorization?: string;
  challenge?: string;
};

async function meetingsRequest<T>(
  relayWsUrl: string,
  method: "GET" | "POST",
  path: string,
  opts: {
    query?: string;
    body?: string;
    hivetalk?: HivetalkHeaders;
    signal?: AbortSignal;
  } = {},
): Promise<T> {
  const { query, body, hivetalk, signal } = opts;
  const url = `${relayHttpBase(relayWsUrl)}${MEETINGS_PREFIX}${path}${
    query ? `?${query}` : ""
  }`;

  const headers: Record<string, string> = {
    Authorization: await buzzNip98(url, method, body),
  };
  if (method === "POST") headers["Content-Type"] = "application/json";
  if (hivetalk?.authorization)
    headers["X-Hivetalk-Authorization"] = hivetalk.authorization;
  if (hivetalk?.challenge) headers["X-Hivetalk-Challenge"] = hivetalk.challenge;

  const response = await fetch(url, {
    method,
    headers,
    body: method === "POST" ? (body ?? "") : undefined,
    signal,
  });

  if (!response.ok) {
    const parsed = (await response.json().catch(() => ({}))) as Record<
      string,
      unknown
    >;
    const err: MeetingError = classifyMeetingError(response.status, parsed);
    if (response.status === 409 && typeof parsed.bolt11 === "string") {
      err.pendingInvoice = parsed as unknown as SubscribeIntent;
    }
    throw err;
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Run a full challenge → sign → send cycle for a Pattern 1 endpoint. */
async function challengeFlow<T>(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  args: {
    action: HivetalkAction;
    method: "GET" | "POST";
    localPath: string;
    upstreamPath: string;
    body?: string;
    query?: string;
    signal?: AbortSignal;
  },
): Promise<T> {
  const { action, method, localPath, upstreamPath, body, query, signal } = args;
  const { nonce, challenge } = await hivetalkChallenge(relayWsUrl, signal);
  const authorization = await signHivetalkAction({
    action,
    apiBase: cap.apiBase,
    upstreamPath,
    method,
    rawBody: body ?? "",
    nonce,
    query,
  });
  return meetingsRequest<T>(relayWsUrl, method, localPath, {
    body,
    query,
    hivetalk: { authorization, challenge },
    signal,
  });
}

// --- capability ---

/** Read the relay's advertised Meetings capability from NIP-11 `/info`. */
export async function fetchMeetingsCapability(
  relayWsUrl: string,
  signal?: AbortSignal,
): Promise<RelayMeetingsCapability | null> {
  const response = await fetch(`${relayHttpBase(relayWsUrl)}/info`, {
    headers: { Accept: "application/nostr+json" },
    signal,
  });
  if (!response.ok) return null;
  const info = (await response.json().catch(() => ({}))) as RelayMeetingsInfo;
  return relayMeetingsCapability(info);
}

// --- public API (unauthenticated-to-HiveTalk, buzz-membership-gated) ---

export async function getPlans(
  relayWsUrl: string,
  signal?: AbortSignal,
): Promise<MeetingPlan[]> {
  // HiveTalk answers with `{ free_quota, plans: [...] }`, not a bare array.
  const body = await meetingsRequest<unknown>(relayWsUrl, "GET", "/plans", {
    signal,
  });
  return normalizePlans(body);
}

export async function listRooms(
  relayWsUrl: string,
  signal?: AbortSignal,
): Promise<ActiveRoom[]> {
  const body = await meetingsRequest<unknown>(
    relayWsUrl,
    "GET",
    "/list-rooms",
    {
      signal,
    },
  );
  return normalizeRooms(body);
}

export async function listRoomsByPubkey(
  relayWsUrl: string,
  pubkeyHex: string,
  signal?: AbortSignal,
): Promise<ActiveRoom[]> {
  // `/rooms-by-pubkey` returns registered rooms keyed `room_name`, not the
  // LiveKit `name` that `/list-rooms` uses. See `normalizeRooms`.
  const body = await meetingsRequest<unknown>(
    relayWsUrl,
    "GET",
    "/rooms-by-pubkey",
    { query: `pubkey=${encodeURIComponent(pubkeyHex)}`, signal },
  );
  return normalizeRooms(body);
}

// --- public API (HiveTalk challenge flow) ---

export function getSubscription(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  signal?: AbortSignal,
): Promise<SubscriptionStatus> {
  return challengeFlow(relayWsUrl, cap, {
    action: "subscription",
    method: "GET",
    localPath: "/subscription",
    upstreamPath: "/api/subscription",
    signal,
  });
}

export function subscribe(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  plan: string,
  signal?: AbortSignal,
): Promise<SubscribeIntent> {
  const body = JSON.stringify({ plan });
  return challengeFlow(relayWsUrl, cap, {
    action: "subscribe",
    method: "POST",
    localPath: "/subscribe",
    upstreamPath: "/api/subscribe",
    body,
    signal,
  });
}

export function getPaymentStatus(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  intentId: string,
  signal?: AbortSignal,
): Promise<PaymentStatus> {
  const query = `id=${encodeURIComponent(intentId)}`;
  return challengeFlow(relayWsUrl, cap, {
    action: "payment-status",
    method: "GET",
    localPath: "/payment/status",
    upstreamPath: "/api/payment/status",
    query,
    signal,
  });
}

export function registerRoom(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  roomName: string,
  signal?: AbortSignal,
): Promise<RegisteredRoom> {
  // HiveTalk's `/api/register-room` requires `room_name` (snake_case) and
  // answers `400 room_name is required` to anything else.
  const body = JSON.stringify({ room_name: roomName });
  return challengeFlow(relayWsUrl, cap, {
    action: "create-room",
    method: "POST",
    localPath: "/register-room",
    upstreamPath: "/api/register-room",
    body,
    signal,
  });
}

// --- public API (HiveTalk Pattern 2 body-signed) ---

export async function getMeetingToken(
  relayWsUrl: string,
  cap: Pick<RelayMeetingsCapability, "apiBase">,
  roomName: string,
  participantName: string,
  pubkeyHex: string,
  signal?: AbortSignal,
): Promise<MeetingToken> {
  const body = await buildGetTokenBody(
    cap.apiBase,
    roomName,
    participantName,
    pubkeyHex,
  );
  return meetingsRequest(relayWsUrl, "POST", "/get-token", { body, signal });
}

// --- public API (moderation — LiveKit JWT, used by Phase 4.3) ---

export type ModerationAction =
  | "kick-user"
  | "mute-user"
  | "room/stage/promote"
  | "room/stage/demote"
  | "room/moderator/promote"
  | "room/moderator/demote"
  | "room/notify-lock"
  | "room/mute-on-join"
  | "room/audience-mode"
  | "room/delete";

/**
 * Call a moderation endpoint with the LiveKit JWT from `getMeetingToken`.
 * `payload` is forwarded as the raw JSON body, byte for byte — the relay proxy
 * cannot re-serialize it (HiveTalk signs `sha256(rawBody)`), so whatever the
 * caller passes is exactly what HiveTalk validates.
 *
 * Shapes are pinned to HiveTalk's `openapi.yaml` (archived at
 * `RESEARCH/HIVETALK_OPENAPI.yaml`), **not** to Phase 4.3, which specified
 * endpoints and auth but no body fields:
 *
 * - `kick-user`, `mute-user` → `ParticipantAction`
 *   `{ roomName, participantIdentity }` (both required)
 * - `room/notify-lock`, `room/mute-on-join` → `RoomToggle`
 *   `{ roomName, enabled }` (both required)
 *
 * camelCase here is correct and is not an oversight: these endpoints are
 * LiveKit-backed, while the registry endpoints are snake_case (`registerRoom`
 * sends `room_name`). Build the bodies with `moderationPayloads.ts` rather than
 * inline object literals, so the shape has exactly one definition.
 */
export function moderateRoom(
  relayWsUrl: string,
  action: ModerationAction,
  livekitJwt: string,
  payload: Record<string, unknown>,
  signal?: AbortSignal,
): Promise<void> {
  return meetingsRequest(relayWsUrl, "POST", `/${action}`, {
    body: JSON.stringify(payload),
    hivetalk: { authorization: `Bearer ${livekitJwt}` },
    signal,
  });
}
