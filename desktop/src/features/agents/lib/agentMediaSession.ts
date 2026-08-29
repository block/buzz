import type { RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * The `provider` values this client knows how to connect to.
 *
 * Mirrors `KNOWN_MEDIA_PROVIDERS` in the relay's ingest validation. The relay
 * refuses to store an announcement naming anything else, so an unknown value
 * here means the relay is newer than the client — render nothing rather than
 * guessing at a transport.
 */
export const SUPPORTED_MEDIA_PROVIDERS = ["livekit"] as const;

export type MediaProvider = (typeof SUPPORTED_MEDIA_PROVIDERS)[number];

/** Track kinds a session may carry. `avatar_video` is an agent's rendered face. */
export type MediaTrackKind = "avatar_video" | "camera" | "screen" | "audio";

export type MediaSessionParticipant = {
  pubkey: string;
  tracks: MediaTrackKind[];
};

/**
 * A live agent media session, parsed from a kind:48200 announcement.
 *
 * `viewer` is what *this* client may do in the room. v1 announcements grant
 * subscribe-only video plus outbound audio; screen share and camera arrive in
 * later versions without a wire change.
 */
export type AgentMediaSession = {
  /** The 48200 event id — the anchor an eventual 48201 references. */
  eventId: string;
  /** Whose session this is. The announcement's signer. */
  agentPubkey: string;
  channelId: string;
  /** The message that prompted the session, when there was one. */
  sourceEventId: string | null;
  provider: MediaProvider;
  serverUrl: string;
  room: string;
  /** Where to exchange this viewer's identity for a short-lived room token. */
  tokenEndpoint: string | null;
  participants: MediaSessionParticipant[];
  viewer: {
    subscribe: MediaTrackKind[];
    publish: MediaTrackKind[];
  };
  startedAt: number;
  /**
   * Unix seconds after which this session stops being worth rendering.
   *
   * A presentation expiry, not a lease — it reaps nothing on the provider. It
   * exists because the 48201 that would retire this card may never arrive: an
   * agent that crashes publishes no end event, and the card would otherwise
   * advertise a dead room until the channel is reloaded.
   */
  expiresAt: number;
};

const TRACK_KINDS: readonly MediaTrackKind[] = [
  "avatar_video",
  "camera",
  "screen",
  "audio",
];

function asTrackKinds(value: unknown): MediaTrackKind[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is MediaTrackKind =>
    TRACK_KINDS.includes(entry as MediaTrackKind),
  );
}

function firstTagValue(event: RelayEvent, name: string): string | null {
  for (const tag of event.tags ?? []) {
    if (tag[0] === name && typeof tag[1] === "string" && tag[1].length > 0) {
      return tag[1];
    }
  }
  return null;
}

/**
 * Parse a kind:48200 announcement into a session, or null if unusable.
 *
 * Returns null rather than throwing: an announcement is untrusted input from
 * another member, and one malformed event must not take the panel down. The
 * relay validates the same fields at ingest, so a null here means either a
 * relay that skipped validation or a provider this client is too old to know.
 */
export function parseAgentMediaSession(
  event: RelayEvent,
): AgentMediaSession | null {
  const channelId = firstTagValue(event, "h");
  if (!channelId) return null;

  let body: unknown;
  try {
    body = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (typeof body !== "object" || body === null) return null;
  const record = body as Record<string, unknown>;

  const provider = record.provider;
  if (
    typeof provider !== "string" ||
    !SUPPORTED_MEDIA_PROVIDERS.includes(provider as MediaProvider)
  ) {
    return null;
  }

  const connect = record.connect;
  if (typeof connect !== "object" || connect === null) return null;
  const { url, room } = connect as Record<string, unknown>;
  if (typeof url !== "string" || typeof room !== "string") return null;
  if (url.length === 0 || room.length === 0) return null;

  // The relay already rejects a non-http(s) token endpoint, but this client
  // must not depend on that: an announcement can also arrive from a relay
  // running an older ingest. Drop the endpoint rather than the session — a
  // session with no endpoint is simply one this viewer cannot get a token for.
  const rawEndpoint = record.token_endpoint;
  const tokenEndpoint =
    typeof rawEndpoint === "string" &&
    (rawEndpoint.startsWith("https://") || rawEndpoint.startsWith("http://"))
      ? rawEndpoint
      : null;

  const participants: MediaSessionParticipant[] = Array.isArray(
    record.participants,
  )
    ? record.participants.flatMap((entry) => {
        if (typeof entry !== "object" || entry === null) return [];
        const { pubkey, tracks } = entry as Record<string, unknown>;
        if (typeof pubkey !== "string") return [];
        return [
          { pubkey: normalizePubkey(pubkey), tracks: asTrackKinds(tracks) },
        ];
      })
    : [];

  const viewer =
    typeof record.viewer === "object" && record.viewer !== null
      ? (record.viewer as Record<string, unknown>)
      : {};

  // Fail closed on a missing or unusable expiry rather than defaulting to one.
  // Any default is a guess about how long a room this client cannot see stays
  // alive, and the generous guess is the one that keeps a dead card on screen.
  // The relay requires the field, so a body without it is an older relay.
  const expiresAt = record.expires_at;
  if (typeof expiresAt !== "number" || !Number.isSafeInteger(expiresAt)) {
    return null;
  }
  if (expiresAt <= event.created_at) return null;

  return {
    eventId: event.id,
    // Ownership is the signature, not a tag — the relay enforces that an agent
    // announces only its own session (or the relay ends one, naming the agent).
    agentPubkey: normalizePubkey(event.pubkey),
    channelId,
    sourceEventId: firstTagValue(event, "e"),
    provider: provider as MediaProvider,
    serverUrl: url,
    room,
    tokenEndpoint,
    participants,
    viewer: {
      subscribe: asTrackKinds(viewer.subscribe),
      publish: asTrackKinds(viewer.publish),
    },
    startedAt: event.created_at,
    expiresAt,
  };
}

/** A kind:48201 event, reduced to what deciding whether to honour it needs. */
export type AgentMediaSessionEnd = {
  /** The 48200 event id this end closes. */
  startEventId: string;
  /** Who signed the end. */
  signer: string;
  /** The single `p` tag, when there is exactly one — the relay-signed shape. */
  subject: string | null;
};

/** Parse a kind:48201 event, or null if it names no start. */
export function parseAgentMediaSessionEnd(
  event: RelayEvent,
): AgentMediaSessionEnd | null {
  const startEventId = firstTagValue(event, "e");
  if (!startEventId) return null;

  const pTags = (event.tags ?? [])
    .filter((tag) => tag[0] === "p" && typeof tag[1] === "string")
    .map((tag) => normalizePubkey(tag[1] as string));

  return {
    startEventId,
    signer: normalizePubkey(event.pubkey),
    subject: pTags.length === 1 ? pTags[0] : null,
  };
}

/**
 * Whether `end` may retire `session`.
 *
 * Two shapes and no others, matching the relay's rule: the owner ends its own
 * session, or the relay ends one and names the owner in a single `p` tag.
 *
 * This client cannot verify that the second shape really came from the relay —
 * it does not know the relay's pubkey. It does not have to: the relay rejects a
 * `p` tag naming anyone but the signer unless the signer is the relay itself,
 * so no third party can produce this shape. Honouring it here admits nothing a
 * relay running this ingest would have stored anyway, and the first shape —
 * signer equals owner — is checked outright.
 *
 * Without either check, any member could retire another agent's live card by
 * publishing a 48201 naming its start.
 */
export function endRetiresSession(
  end: AgentMediaSessionEnd,
  session: AgentMediaSession,
): boolean {
  if (end.startEventId !== session.eventId) return false;
  return (
    end.signer === session.agentPubkey || end.subject === session.agentPubkey
  );
}

/** Whether `session` has passed its announced expiry at `nowSeconds`. */
export function isSessionExpired(
  session: AgentMediaSession,
  nowSeconds: number,
): boolean {
  return session.expiresAt <= nowSeconds;
}
