const LEASE_PREFIX = "<!-- buzz-artillery-referee-lease:";
const LEASE_SUFFIX = " -->";
const LEASE_PATTERN = /<!-- buzz-artillery-referee-lease:(%7B[\s\S]*?%7D) -->/;

export type ArtilleryRefereeLeaseEvent = {
  action: "claim" | "renew" | "release";
  expiresAt: number;
  issuedAt: number;
  matchId: string;
  ownerId: string;
  term: number;
  type: "buzz.game.artillery.referee-lease.v1";
  version: 1;
};

export type ArtilleryRefereeLease = {
  active: boolean;
  expiresAt: number;
  ownerId: string;
  term: number;
};

function isLeaseAction(
  value: unknown,
): value is ArtilleryRefereeLeaseEvent["action"] {
  return value === "claim" || value === "renew" || value === "release";
}

/** Embeds a referee lease record in a human-readable thread reply. */
export function formatArtilleryRefereeLeaseMessage(
  event: ArtilleryRefereeLeaseEvent,
) {
  const copy =
    event.action === "claim"
      ? `🛡️ Referee lease claimed · term ${event.term}.`
      : event.action === "release"
        ? `🛡️ Referee lease released · term ${event.term}.`
        : `🛡️ Referee lease renewed · term ${event.term}.`;
  return `${copy}\n\n${LEASE_PREFIX}${encodeURIComponent(JSON.stringify(event))}${LEASE_SUFFIX}`;
}

/** Parses a supported referee lease marker from a channel message. */
export function parseArtilleryRefereeLeaseEvent(
  content: string,
): ArtilleryRefereeLeaseEvent | null {
  const encoded = content.match(LEASE_PATTERN)?.[1];
  if (!encoded) return null;
  try {
    const value: unknown = JSON.parse(decodeURIComponent(encoded));
    if (!value || typeof value !== "object") return null;
    const lease = value as Partial<ArtilleryRefereeLeaseEvent>;
    if (
      lease.type !== "buzz.game.artillery.referee-lease.v1" ||
      lease.version !== 1 ||
      !isLeaseAction(lease.action) ||
      typeof lease.matchId !== "string" ||
      typeof lease.ownerId !== "string" ||
      !Number.isInteger(lease.term) ||
      typeof lease.issuedAt !== "number" ||
      typeof lease.expiresAt !== "number"
    ) {
      return null;
    }
    return lease as ArtilleryRefereeLeaseEvent;
  } catch {
    return null;
  }
}

/** Creates a claim, renewal, or release for one lease term. */
export function createArtilleryRefereeLeaseEvent({
  action,
  leaseMs,
  matchId,
  ownerId,
  term,
  now = Date.now(),
}: {
  action: ArtilleryRefereeLeaseEvent["action"];
  leaseMs: number;
  matchId: string;
  ownerId: string;
  term: number;
  now?: number;
}): ArtilleryRefereeLeaseEvent {
  return {
    action,
    expiresAt: action === "release" ? now : now + leaseMs,
    issuedAt: now,
    matchId,
    ownerId,
    term,
    type: "buzz.game.artillery.referee-lease.v1",
    version: 1,
  };
}

/** Elects the lowest owner id among claims in the newest term. */
export function recoverArtilleryRefereeLease(
  events: readonly ArtilleryRefereeLeaseEvent[],
  matchId: string,
  now = Date.now(),
): ArtilleryRefereeLease | null {
  const matching = events.filter((event) => event.matchId === matchId);
  const term = Math.max(0, ...matching.map((event) => event.term));
  if (term === 0) return null;
  const termEvents = matching.filter((event) => event.term === term);
  const ownerId = termEvents
    .filter((event) => event.action === "claim")
    .map((event) => event.ownerId)
    .sort()[0];
  if (!ownerId) return null;
  const ownerEvents = termEvents
    .filter((event) => event.ownerId === ownerId)
    .sort((left, right) => left.issuedAt - right.issuedAt);
  const latest = ownerEvents.at(-1);
  if (!latest) return null;
  return {
    active: latest.action !== "release" && latest.expiresAt > now,
    expiresAt: latest.expiresAt,
    ownerId,
    term,
  };
}

/** Lease duration, shortened only by the desktop E2E test seam. */
export function artilleryRefereeLeaseMs() {
  const override = (
    window as typeof window & { __BUZZ_E2E_ARTILLERY_LEASE_MS__?: number }
  ).__BUZZ_E2E_ARTILLERY_LEASE_MS__;
  return typeof override === "number" && override >= 1_000 ? override : 12_000;
}
