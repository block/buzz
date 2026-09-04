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
   * `https://l402relay.exe.xyz`. The client needs it to build the `u` tag of
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
  /**
   * Open-ended because HiveTalk names some fields two ways and `normalizePlans`
   * reconciles them. The relay allowlists `/plans` (envelope *and* entries), so
   * a field HiveTalk adds without the relay knowing about it never arrives.
   */
  [key: string]: unknown;
};

/**
 * Raw `GET /api/plans` body. HiveTalk wraps the list in an envelope and names
 * its fields `id` / `price_sats`, not `plan` / `amount_sats`; `normalizePlans`
 * is the single place that reconciles the two.
 */
type PlansResponse = {
  free_quota?: number;
  plans?: unknown;
};

function planPeriod(days: unknown): string | undefined {
  if (typeof days !== "number" || !Number.isFinite(days)) return undefined;
  if (days === 365) return "year";
  if (days === 730) return "2 years";
  return `${days} days`;
}

/**
 * Map the HiveTalk `/api/plans` envelope onto `MeetingPlan[]`.
 *
 * Accepts a bare array too, so a future HiveTalk that drops the envelope does
 * not blank the plan grid. Entries missing an id or a price are dropped rather
 * than rendered as `undefined sats`.
 */
export function normalizePlans(body: unknown): MeetingPlan[] {
  const raw = Array.isArray(body)
    ? body
    : Array.isArray((body as PlansResponse | null)?.plans)
      ? ((body as PlansResponse).plans as unknown[])
      : [];

  return raw.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) return [];
    const record = entry as Record<string, unknown>;
    const plan = record.plan ?? record.id;
    const amount = record.amount_sats ?? record.price_sats;
    if (typeof plan !== "string" || typeof amount !== "number") return [];
    const period = record.period ?? record.interval ?? planPeriod(record.days);
    return [
      {
        ...record,
        plan,
        amount_sats: amount,
        ...(period === undefined ? {} : { period }),
      } satisfies MeetingPlan,
    ];
  });
}

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
  /** Unix seconds from HiveTalk; ISO 8601 tolerated. See `expiryMs`. */
  expires_at: string | number;
  status: string;
};

export type PaymentStatus = {
  intent_id: string;
  status: string;
  plan?: string;
  settled_msat?: number;
  /** Unix seconds from HiveTalk; ISO 8601 tolerated. See `expiryMs`. */
  expires_at?: string | number;
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

export type ActiveRoom = {
  name: string;
  numParticipants?: number;
  locked?: boolean;
  lobby_enabled?: boolean;
  room_kind?: string;
};

/**
 * Map a room list onto `ActiveRoom[]`.
 *
 * Two upstream endpoints feed this type and they do **not** agree:
 * `/api/list-rooms` reports live LiveKit rooms (`name`, `numParticipants`),
 * while `/api/rooms-by-pubkey` reports registered rooms (`room_name`,
 * `room_id`, no participant count). Reading only `name` left every "My rooms"
 * entry blank with an undefined join target. Entries with no usable name are
 * dropped rather than rendered as an unjoinable blank row.
 */
export function normalizeRooms(body: unknown): ActiveRoom[] {
  if (!Array.isArray(body)) return [];
  return body.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) return [];
    const record = entry as Record<string, unknown>;
    const name = record.name ?? record.room_name;
    if (typeof name !== "string" || name.trim().length === 0) return [];
    const participants = record.numParticipants ?? record.num_participants;
    return [
      {
        ...record,
        name,
        ...(typeof participants === "number"
          ? { numParticipants: participants }
          : {}),
      } satisfies ActiveRoom,
    ];
  });
}

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

/**
 * The rejection every Meetings query/mutation raises when the relay never
 * advertised `buzz-meetings`. Here rather than in `hooks.ts` so the wording
 * lives with the rest of the friendly copy.
 */
export function notConfiguredError(): MeetingError {
  return new MeetingError("not_configured", 0, FRIENDLY.not_configured);
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
      // The relay 404s every `/meetings/*` route when the community relay has
      // no HiveTalk config. Any other 404 came from HiveTalk itself —
      // `/api/room-info` answers a name that is not in the registry with a
      // plain-text `404 Room not found`, which the relay forwards verbatim.
      //
      // Caveat for a future `/room-info` caller: on `l402relay` a 404 there is
      // NOT proof the room is unregistered. `buzz-meet-control` is returned by
      // `rooms-by-pubkey` and still 404s on `room-info` — it is the one registry
      // row with no `identifier`. Treat `rooms-by-pubkey` as the authority for
      // registration and a `room-info` 404 as "no detail available". Nothing
      // calls `/room-info` today, which is why this stays a note.
      if (reason === "meetings is not configured")
        return pick("not_configured");
      return pick("room_not_registered");
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
