import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_AGENT_MEDIA_SESSION_ENDED,
  KIND_AGENT_MEDIA_SESSION_STARTED,
} from "@/shared/constants/kinds";
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

/**
 * A 64-char lowercase-hex run inside a provider identity.
 *
 * Every identity this room mints for a *person* is exactly one of these: the
 * gateway joins a viewer under its own Buzz pubkey. An identity carrying a
 * pubkey is therefore naming a specific participant, and the test below is
 * reached only once that pubkey is known not to be the agent's.
 */
const PUBKEY_IN_IDENTITY = /[0-9a-f]{64}/;

/**
 * Whether a track published by `participantIdentity` is this session's agent's.
 *
 * Called for every subscribed track, because only the announcing agent's face
 * and voice belong in its panel. Getting this wrong is not a cosmetic slip: the
 * room hook holds one audio track, so admitting somebody else's voice *detaches
 * the agent's*. One member unmuting would silence the agent for another and
 * speak under its face.
 *
 * Three rules, in order:
 *
 * 1. The provider identity contains the agent's hex pubkey. Unambiguous, and
 *    the only rule that proves anything about who actually published.
 * 2. Otherwise, an identity carrying some *other* pubkey is refused outright.
 *    Viewers join as their own pubkey and a v1 announcement grants them a
 *    microphone, so this is the case that genuinely occurs.
 * 3. Only then may the announcement's declaration settle it: exactly one
 *    declared publisher of this track kind, and that publisher is the agent.
 *
 * Rule 3 cannot be dropped in favour of rule 1, tempting as that is. No gateway
 * is obliged to put a pubkey in its provider identity, and the one this was
 * built against does not — the Anam avatar worker joins under an identity of
 * Anam's choosing, and the gateway's `agent_token` helper, whose docstring
 * calls the identity contract load-bearing, has no callers. Requiring rule 1
 * would leave every session today faceless.
 *
 * Audio is checked against its own declaration rather than video's: an
 * announcement may name one avatar publisher and several audio ones, and the
 * agent's face arriving unambiguously says nothing about whose voice this is.
 */
export function trackBelongsToSessionAgent(
  session: AgentMediaSession,
  participantIdentity: string,
  kind: "audio" | "video",
): boolean {
  // `session.agentPubkey` is normalized at parse time; the identity is not.
  const identity = participantIdentity.toLowerCase();
  if (identity.includes(session.agentPubkey)) return true;
  if (PUBKEY_IN_IDENTITY.test(identity)) return false;

  const declared: MediaTrackKind = kind === "video" ? "avatar_video" : "audio";
  const publishers = session.participants.filter((entry) =>
    entry.tracks.includes(declared),
  );
  // The sole declared publisher has to be the agent. Counting alone admits an
  // announcement naming somebody else as the only voice — which is not an
  // ambiguity to resolve in the agent's favour, but a statement that the voice
  // is not the agent's.
  return (
    publishers.length === 1 && publishers[0].pubkey === session.agentPubkey
  );
}

/**
 * Whether two folds produced the same live set, element identity included.
 *
 * Elements are compared by reference, which is exactly the question: the fold
 * reuses a previously parsed session for an unchanged event, so a differing
 * reference means the content differed too.
 */
function sessionsUnchanged(
  a: readonly AgentMediaSession[],
  b: readonly AgentMediaSession[],
): boolean {
  if (a.length !== b.length) return false;
  for (let index = 0; index < a.length; index += 1) {
    if (!Object.is(a[index], b[index])) return false;
  }
  return true;
}

/**
 * Fold lifecycle events into a channel's live sessions, newest first.
 *
 * A fold over everything seen rather than an incremental update: lifecycle
 * events are rare, replay on reconnect is common, and refolding is correct
 * regardless of arrival order — an end that arrives before its start still
 * retires that start. A session leaves the set two ways, an end event its owner
 * was entitled to publish, or its own announced expiry passing. The second
 * exists because the first may never happen: an agent that crashes publishes no
 * 48201.
 *
 * **Identity-preserving, and that is a correctness requirement rather than an
 * optimisation.** `useAgentMediaRoom` keys a whole WebRTC connection on the
 * session object, so handing it a fresh object for an unchanged session tears
 * the call down, fetches another viewer token and rejoins. Refolding on every
 * arrival used to do exactly that: an unrelated agent starting or ending a
 * session — or any expiry firing — interrupted an open call, and each replayed
 * history event cost its own token request.
 *
 * Two guarantees carry that. A 48200 is immutable, so a session already parsed
 * under an event id can only parse to the same value and the object from
 * `previous` is reused. And when the outcome matches `previous` entirely,
 * `previous` itself is returned, so `setState` bails out instead of
 * re-rendering every consumer.
 */
export function foldLiveSessions(
  events: Iterable<RelayEvent>,
  previous: readonly AgentMediaSession[],
  nowSeconds: number,
): readonly AgentMediaSession[] {
  const started: AgentMediaSession[] = [];
  const ends: AgentMediaSessionEnd[] = [];
  const parsed = new Map(previous.map((session) => [session.eventId, session]));

  for (const event of events) {
    if (event.kind === KIND_AGENT_MEDIA_SESSION_STARTED) {
      const already = parsed.get(event.id);
      if (already) {
        started.push(already);
        continue;
      }
      const session = parseAgentMediaSession(event);
      if (session) started.push(session);
      continue;
    }
    if (event.kind === KIND_AGENT_MEDIA_SESSION_ENDED) {
      const end = parseAgentMediaSessionEnd(event);
      if (end) ends.push(end);
    }
  }

  const live = started
    .filter(
      (session) =>
        // Check the ender's standing rather than matching on the event id
        // alone: without it any member could retire another agent's live card
        // by publishing a 48201 that names its start.
        !ends.some((end) => endRetiresSession(end, session)) &&
        !isSessionExpired(session, nowSeconds),
    )
    .sort((a, b) => b.startedAt - a.startedAt);

  return sessionsUnchanged(previous, live) ? previous : live;
}
