/**
 * Meetings (HiveTalk / LiveKit video) — pure types and helpers.
 *
 * No I/O lives here. The relay-through client is in `./relay.ts`. Field sets
 * mirror the relay's allowlist filter in `crates/buzz-relay/src/api/meetings.rs`
 * exactly: the relay strips everything else before the client sees it, so the
 * shapes below are deliberately small and every non-identifying field is
 * optional.
 */

/** NIP-11 `/info` shape this feature cares about. */
export type RelayMeetingsInfo = {
  meetings?: {
    provider?: string;
    proxy?: string;
    api_base?: string;
  };
  supported_extensions?: string[];
};

/** The relay's advertised, safe-to-use Meetings capability. */
export type RelayMeetingsCapability = {
  /** Relay-relative path prefix, e.g. `/meetings`. */
  proxyPrefix: string;
  /**
   * Public HiveTalk API root the relay forwards to, e.g.
   * `https://premrelay.exe.xyz`. The client needs it to build the `u` tag of
   * its own HiveTalk-signed request — HiveTalk verifies that signature against
   * the upstream URL, not the relay URL.
   */
  apiBase: string;
};

/**
 * Same relay-path safety rules as `relayKlipyCapability` (see
 * `features/gifs/api.ts`): must be root-relative, single-segment-safe, and free
 * of traversal / escape / query / fragment characters.
 */
function safeRelayPath(path: unknown): path is string {
  return (
    typeof path === "string" &&
    path.startsWith("/") &&
    !path.startsWith("//") &&
    !path.includes("\\") &&
    !path.includes("%") &&
    !path.includes("?") &&
    !path.includes("#") &&
    !path.split("/").some((segment) => segment === "." || segment === "..")
  );
}

function isHttpsUrl(value: unknown): value is string {
  if (typeof value !== "string" || value.length === 0) return false;
  try {
    return new URL(value).protocol === "https:";
  } catch {
    return false;
  }
}

/** The safe relay-advertised Meetings capability, or `null` when unavailable. */
export function relayMeetingsCapability(
  info: RelayMeetingsInfo,
): RelayMeetingsCapability | null {
  const proxyPrefix = info.meetings?.proxy;
  const apiBase = info.meetings?.api_base;
  if (
    info.supported_extensions?.includes("buzz-meetings") === true &&
    info.meetings?.provider === "hivetalk" &&
    safeRelayPath(proxyPrefix) &&
    isHttpsUrl(apiBase)
  ) {
    // Normalize away any trailing slash so callers can concatenate freely.
    return { proxyPrefix, apiBase: apiBase.replace(/\/+$/, "") };
  }
  return null;
}

// --- Domain types (relay allowlist mirrors) ---

export type MeetingPlan = {
  plan: string;
  amount_sats: number;
  room_quota?: number;
  can_record?: boolean;
  /** HiveTalk may add fields; the relay passes `/plans` through unfiltered. */
  [key: string]: unknown;
};

export type SubscriptionStatus = {
  status: string;
  entitled: boolean;
  room_quota: number;
  rooms_in_use: number;
  free_quota: number;
  grace_days: number;
  in_grace: boolean;
  paid_until: string | null;
  plan: string | null;
  can_record: boolean;
  pubkey: string;
};

export type SubscribeIntent = {
  intent_id: string;
  plan: string;
  amount_sats: number;
  bolt11: string;
  payment_hash: string;
  expires_at: string;
  status: string;
};

export type PaymentStatus = {
  intent_id: string;
  status: string;
  plan?: string;
  settled_msat?: number;
  expires_at?: string;
  subscription?: SubscriptionStatus;
};

export type RegisteredRoom = {
  room_id: string;
  room_name: string;
  pubkey: string;
  identifier?: string;
  locked?: boolean;
  lobby_enabled?: boolean;
  mute_on_join?: boolean;
  room_description?: string;
  room_picture_url?: string;
  created_via?: string;
  broadcast_pending?: boolean;
  status?: string;
  updated_at?: string;
};

export type RoomInfo = {
  room_name: string;
  room_id?: string;
  is_private?: boolean;
  room_kind?: string;
};

export type ActiveRoom = {
  name: string;
  numParticipants?: number;
  locked?: boolean;
  lobby_enabled?: boolean;
  room_kind?: string;
};

export type MeetingToken = { token: string; url: string };

// --- Error taxonomy ---

export type MeetingErrorKind =
  | "subscription_required"
  | "subscription_expired"
  | "room_not_registered"
  | "ephemeral_rooms_are_dashboard_only"
  | "pending_invoice"
  | "not_configured"
  | "rate_limited"
  | "provider_unavailable"
  | "membership_required"
  | "bad_signature"
  | "unknown";

const FRIENDLY: Record<MeetingErrorKind, string> = {
  subscription_required:
    "The room host needs an active HiveTalk subscription to open this room.",
  subscription_expired: "The host's HiveTalk subscription has expired.",
  room_not_registered: "That room isn't registered.",
  ephemeral_rooms_are_dashboard_only:
    "Ephemeral rooms can't be opened from Buzz.",
  pending_invoice: "You already have a pending invoice for a subscription.",
  not_configured: "Meetings isn't enabled on this relay.",
  rate_limited:
    "Too many meeting actions — please wait a moment and try again.",
  provider_unavailable:
    "The meeting provider is unavailable right now. Try again shortly.",
  membership_required: "Join this community to use Meetings.",
  bad_signature: "Your session signature was rejected. Try again.",
  unknown: "The meeting request failed.",
};

export type MeetingErrorBody = {
  error?: string;
  reason?: string;
  subscribe_url?: string;
  plans?: unknown;
};

export class MeetingError extends Error {
  readonly kind: MeetingErrorKind;
  readonly status: number;
  /** Populated on a 409 (`pending_invoice`) when the body parses as an intent. */
  pendingInvoice?: SubscribeIntent;
  /** Parsed from a 429 message, in seconds, when present. */
  retryAfterSecs?: number;

  constructor(
    kind: MeetingErrorKind,
    status: number,
    message: string,
    options?: { cause?: unknown },
  ) {
    super(message, options);
    this.name = "MeetingError";
    this.kind = kind;
    this.status = status;
  }
}

function retryAfterFromMessage(
  message: string | undefined,
): number | undefined {
  const match = message?.match(/retry in (\d+)s/i);
  return match ? Number(match[1]) : undefined;
}

/** Map an HTTP status + relay/HiveTalk error body onto a typed `MeetingError`. */
export function classifyMeetingError(
  status: number,
  body: MeetingErrorBody,
): MeetingError {
  const reason = body.reason ?? body.error;

  const pick = (kind: MeetingErrorKind): MeetingError =>
    new MeetingError(kind, status, FRIENDLY[kind]);

  if (reason === "relay_membership_required")
    return pick("membership_required");

  switch (status) {
    case 401:
      return pick("bad_signature");
    case 402:
      return pick(
        reason === "subscription_expired"
          ? "subscription_expired"
          : "subscription_required",
      );
    case 403:
      if (reason === "room_not_registered") return pick("room_not_registered");
      if (reason === "ephemeral_rooms_are_dashboard_only")
        return pick("ephemeral_rooms_are_dashboard_only");
      if (reason === "subscription_expired")
        return pick("subscription_expired");
      return pick("membership_required");
    case 404:
      return pick("not_configured");
    case 409:
      return pick("pending_invoice");
    case 429: {
      const err = pick("rate_limited");
      err.retryAfterSecs = retryAfterFromMessage(body.error);
      return err;
    }
    case 502:
    case 503:
      return pick("provider_unavailable");
    default: {
      const message = body.error || `Meeting request failed (${status})`;
      return new MeetingError("unknown", status, message);
    }
  }
}

// --- LiveKit token claim decode (UI gating only) ---

/**
 * Decode a base64url string, restoring the `=` padding `atob` requires. A raw
 * JWT payload segment is usually not a multiple of 4 characters, and `atob`
 * throws on those — without this pad the owner/moderator claims silently read
 * as `false` and hide host controls from the legitimate owner.
 */
function base64UrlDecode(input: string): string {
  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const padLength = (4 - (normalized.length % 4)) % 4;
  return atob(normalized.padEnd(normalized.length + padLength, "="));
}

/**
 * Decode the `owner` / `moderator` / `room` claims from a LiveKit JWT.
 *
 * **UI gating only.** Never trust this for authorization — the relay and
 * HiveTalk enforce host controls for real. A malformed token yields all-false.
 */
export function decodeMeetingTokenClaims(jwt: string): {
  owner: boolean;
  moderator: boolean;
  room: string | null;
} {
  const fallback = { owner: false, moderator: false, room: null };
  try {
    const [, payload] = jwt.split(".");
    if (!payload) return fallback;
    const json = JSON.parse(base64UrlDecode(payload)) as {
      video?: { room?: string; roomAdmin?: boolean };
      metadata?: string;
      owner?: boolean;
      moderator?: boolean;
      room?: string;
    };
    // HiveTalk (spike S0.2) puts `owner`/`moderator` at the top level; the
    // LiveKit grant nests `room` under `video`. Accept either shape.
    let owner = json.owner === true;
    let moderator = json.moderator === true || json.video?.roomAdmin === true;
    if (typeof json.metadata === "string") {
      try {
        const meta = JSON.parse(json.metadata) as Record<string, unknown>;
        owner ||= meta.owner === true;
        moderator ||= meta.moderator === true;
      } catch {
        // metadata is opaque — ignore
      }
    }
    return {
      owner,
      moderator: moderator || owner,
      room: json.video?.room ?? json.room ?? null,
    };
  } catch {
    return fallback;
  }
}
