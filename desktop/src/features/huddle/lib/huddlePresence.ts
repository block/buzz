import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

type AdmissionState = {
  present: boolean;
  rosterRevision: number | null;
  generation: string | null;
  createdAt: number;
  eventId: string;
};

type LivenessGeneration = string;

type HuddleSession = {
  startCreator: string;
  startCreatedAt: number;
  startEventId: string;
  endState: AdmissionState | null;
  admissionsByParticipant: Map<string, Map<string, AdmissionState>>;
  legacyStateByParticipant: Map<string, AdmissionState>;
  latestRosterRevision: number | null;
  latestRosterCreatedAt: number;
  latestRosterEventId: string;
  generation: LivenessGeneration | null;
  generationIsAuthoritative: boolean;
  departedAdmissions: Map<string, AdmissionState>;
  compactedRosterRevisionFloor: number | null;
};

type LifecycleContent = {
  ephemeralChannelId: string | null;
  rosterRevision: number | null;
  admissionId: string | null;
  generation: string | null;
};

/** Compare monotonic decimal mesh generations; opaque epochs are unordered. */
export function compareHuddleGenerations(
  candidate: LivenessGeneration,
  current: LivenessGeneration,
): number | null {
  if (!/^\d+$/.test(candidate) || !/^\d+$/.test(current)) return null;
  const candidateEpoch = BigInt(candidate);
  const currentEpoch = BigInt(current);
  return candidateEpoch < currentEpoch
    ? -1
    : candidateEpoch > currentEpoch
      ? 1
      : 0;
}

function lifecycleContent(event: RelayEvent): LifecycleContent {
  try {
    const content = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
      roster_revision?: unknown;
      admission_id?: unknown;
      generation?: unknown;
    };
    return {
      ephemeralChannelId:
        typeof content.ephemeral_channel_id === "string" &&
        content.ephemeral_channel_id
          ? content.ephemeral_channel_id
          : null,
      rosterRevision:
        typeof content.roster_revision === "number" &&
        Number.isSafeInteger(content.roster_revision) &&
        content.roster_revision >= 0
          ? content.roster_revision
          : null,
      admissionId:
        typeof content.admission_id === "string" && content.admission_id
          ? content.admission_id
          : null,
      generation:
        typeof content.generation === "string" && content.generation
          ? content.generation
          : null,
    };
  } catch {
    return {
      ephemeralChannelId: null,
      rosterRevision: null,
      admissionId: null,
      generation: null,
    };
  }
}

export function huddleSessionId(event: RelayEvent): string | null {
  return lifecycleContent(event).ephemeralChannelId;
}

export function huddleParentChannelId(event: RelayEvent): string | null {
  const value = event.tags.find((tag) => tag[0] === "h")?.[1] ?? "";
  return value.trim() || null;
}

export function huddleLifecycleGeneration(event: RelayEvent): string | null {
  return lifecycleContent(event).generation;
}

function participantPubkey(event: RelayEvent): string | null {
  const value = event.tags.find((tag) => tag[0] === "p")?.[1] ?? event.pubkey;
  const normalized = normalizePubkey(value);
  return normalized || null;
}

function lifecyclePhase(kind: number): number {
  if (kind === KIND_HUDDLE_STARTED) return 0;
  if (kind === KIND_HUDDLE_ENDED) return 2;
  return 1;
}

export function compareHuddleLifecycleEvents(
  left: RelayEvent,
  right: RelayEvent,
): number {
  const timestampOrder = left.created_at - right.created_at;
  if (timestampOrder !== 0) return timestampOrder;

  const phaseOrder = lifecyclePhase(left.kind) - lifecyclePhase(right.kind);
  if (phaseOrder !== 0) return phaseOrder;

  const leftRevision = lifecycleContent(left).rosterRevision;
  const rightRevision = lifecycleContent(right).rosterRevision;
  if (
    leftRevision !== null &&
    rightRevision !== null &&
    leftRevision !== rightRevision
  ) {
    return leftRevision - rightRevision;
  }

  return left.id.localeCompare(right.id);
}

function isNewerAdmissionState(
  candidate: AdmissionState,
  existing: AdmissionState | undefined,
): boolean {
  if (!existing) return true;
  if (
    candidate.rosterRevision !== null &&
    existing.rosterRevision !== null &&
    candidate.rosterRevision !== existing.rosterRevision
  ) {
    return candidate.rosterRevision > existing.rosterRevision;
  }
  if (candidate.createdAt !== existing.createdAt) {
    return candidate.createdAt > existing.createdAt;
  }
  return candidate.eventId > existing.eventId;
}

function participantHasExplicitPresence(
  session: HuddleSession,
  participant: string,
): boolean {
  const admissions = session.admissionsByParticipant.get(participant);
  if (admissions?.size) {
    return [...admissions.values()].some((admission) => admission.present);
  }
  return session.legacyStateByParticipant.get(participant)?.present ?? false;
}

function participantIsPresent(
  session: HuddleSession,
  participant: string,
): boolean {
  return participantHasExplicitPresence(session, participant);
}

function sessionParticipants(session: HuddleSession): Set<string> {
  const candidates = new Set<string>([
    ...session.admissionsByParticipant.keys(),
    ...session.legacyStateByParticipant.keys(),
  ]);
  return new Set(
    [...candidates].filter((participant) =>
      participantIsPresent(session, participant),
    ),
  );
}

type FetchEvents = (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;

export const HUDDLE_LIFECYCLE_PAGE_LIMIT = 500;

/**
 * Load complete persisted huddle lifecycle history.
 *
 * A backing channel's TTL is not an audio-session lifetime: already-connected
 * participants can remain in a room after the channel is archived, and relay
 * deployments may configure a longer TTL. Pagination therefore continues to
 * the beginning of lifecycle history instead of imposing a client-side time
 * horizon. Inclusive `until` pages are de-duplicated by event id.
 */
export async function fetchHuddleLifecycleHistory(
  fetchEvents: FetchEvents,
  channelIds?: string[],
): Promise<RelayEvent[]> {
  const events = new Map<string, RelayEvent>();
  let until: number | undefined;
  let beforeId: string | undefined;

  for (;;) {
    const page = await fetchEvents({
      kinds: [
        KIND_HUDDLE_STARTED,
        KIND_HUDDLE_PARTICIPANT_JOINED,
        KIND_HUDDLE_PARTICIPANT_LEFT,
        KIND_HUDDLE_ENDED,
      ],
      ...(channelIds?.length ? { "#h": channelIds } : {}),
      ...(until === undefined ? {} : { until }),
      limit: HUDDLE_LIFECYCLE_PAGE_LIMIT,
      ...(beforeId === undefined ? {} : { before_id: beforeId }),
    });
    for (const event of page) events.set(event.id, event);
    if (page.length < HUDDLE_LIFECYCLE_PAGE_LIMIT) break;

    const terminal = [...page]
      .sort((left, right) =>
        left.created_at !== right.created_at
          ? right.created_at - left.created_at
          : left.id.localeCompare(right.id),
      )
      .at(-1);
    if (!terminal) break;
    if (until === terminal.created_at && beforeId === terminal.id) {
      throw new Error(
        "Could not load active huddles: pagination cursor did not advance.",
      );
    }
    until = terminal.created_at;
    beforeId = terminal.id;
  }

  return [...events.values()];
}

/** Incremental, bounded reconstruction of authenticated active huddles. */
export class HuddlePresenceTracker {
  private readonly relaySelf: string;
  private readonly sessions = new Map<string, HuddleSession>();

  constructor(relaySelfPubkey: string | null | undefined) {
    this.relaySelf = normalizePubkey(relaySelfPubkey ?? "");
  }

  apply(event: RelayEvent): boolean {
    if (!this.relaySelf) return false;
    const content = lifecycleContent(event);
    const sessionId = content.ephemeralChannelId;
    if (!sessionId) return false;

    if (event.kind === KIND_HUDDLE_STARTED) {
      const creator = normalizePubkey(event.pubkey);
      if (!creator) return false;
      const existing = this.sessions.get(sessionId);
      if (existing?.startEventId === event.id) return false;
      if (
        existing &&
        (event.created_at < existing.startCreatedAt ||
          (event.created_at === existing.startCreatedAt &&
            event.id > existing.startEventId))
      ) {
        return false;
      }
      this.sessions.set(sessionId, {
        startCreator: creator,
        startCreatedAt: event.created_at,
        startEventId: event.id,
        endState: null,
        admissionsByParticipant: new Map(),
        legacyStateByParticipant: new Map(),
        latestRosterRevision: null,
        latestRosterCreatedAt: 0,
        latestRosterEventId: "",
        generation: null,
        generationIsAuthoritative: false,
        departedAdmissions: new Map(),
        compactedRosterRevisionFloor: null,
      });
      return true;
    }

    const session = this.sessions.get(sessionId);
    if (!session) return false;

    const generation = content.generation;
    if (event.kind === KIND_HUDDLE_ENDED) {
      const signer = normalizePubkey(event.pubkey);
      if (signer !== this.relaySelf && signer !== session.startCreator) {
        return false;
      }
      if (
        generation !== null &&
        session.generation !== null &&
        generation !== session.generation
      ) {
        // Teardown from an older room generation must not end the currently
        // live room. A relay-authenticated JOIN or liveness snapshot is the
        // authoritative boundary between generations.
        return false;
      }
      if (generation !== null) session.generation = generation;
      const nextEnd: AdmissionState = {
        present: false,
        rosterRevision: null,
        generation: content.generation,
        createdAt: event.created_at,
        eventId: event.id,
      };
      if (!isNewerAdmissionState(nextEnd, session.endState ?? undefined)) {
        return false;
      }
      session.endState = nextEnd;
      session.admissionsByParticipant.clear();
      session.legacyStateByParticipant.clear();
      session.departedAdmissions.clear();
      return true;
    }

    if (session.endState) return false;
    if (
      event.kind !== KIND_HUDDLE_PARTICIPANT_JOINED &&
      event.kind !== KIND_HUDDLE_PARTICIPANT_LEFT
    ) {
      return false;
    }
    if (normalizePubkey(event.pubkey) !== this.relaySelf) return false;

    const participant = participantPubkey(event);
    if (!participant) return false;

    if (
      generation !== null &&
      session.generation !== null &&
      generation !== session.generation
    ) {
      const generationOrder = compareHuddleGenerations(
        generation,
        session.generation,
      );
      if (session.generationIsAuthoritative && generationOrder !== 1) {
        return false;
      }
      if (event.kind === KIND_HUDDLE_PARTICIPANT_LEFT) return false;
      // Mesh generations are monotonic. An authenticated higher-generation JOIN
      // may supersede liveness; opaque non-mesh epochs require a fresh snapshot.
      session.admissionsByParticipant.clear();
      session.legacyStateByParticipant.clear();
      session.departedAdmissions.clear();
      session.compactedRosterRevisionFloor = null;
    }

    const next: AdmissionState = {
      present: event.kind === KIND_HUDDLE_PARTICIPANT_JOINED,
      rosterRevision: content.rosterRevision,
      generation: content.generation,
      createdAt: event.created_at,
      eventId: event.id,
    };

    const isAfterLatestRosterEvent =
      event.created_at > session.latestRosterCreatedAt ||
      (event.created_at === session.latestRosterCreatedAt &&
        event.id > session.latestRosterEventId);
    if (
      content.rosterRevision !== null &&
      session.latestRosterRevision !== null &&
      content.rosterRevision < session.latestRosterRevision &&
      isAfterLatestRosterEvent &&
      (generation === null ||
        session.generation === null ||
        generation !== session.generation) &&
      (session.compactedRosterRevisionFloor === null ||
        content.rosterRevision > session.compactedRosterRevisionFloor)
    ) {
      // Relay roster revisions are process-local. A strictly lower revision
      // arriving after the latest authenticated roster event starts a new room
      // generation, so admissions from the previous relay process are dead.
      // Equal revisions are valid: a remote join publishes a post-admission
      // snapshot revision that can match a concurrent local mutation.
      session.admissionsByParticipant.clear();
      session.legacyStateByParticipant.clear();
      session.departedAdmissions.clear();
      session.compactedRosterRevisionFloor = null;
    }
    if (generation !== null) session.generation = generation;

    if (
      !next.present &&
      next.rosterRevision !== null &&
      session.compactedRosterRevisionFloor !== null &&
      next.rosterRevision <= session.compactedRosterRevisionFloor
    ) {
      return false;
    }

    if (content.admissionId) {
      const admissions =
        session.admissionsByParticipant.get(participant) ??
        new Map<string, AdmissionState>();
      const departed = session.departedAdmissions.get(content.admissionId);
      const existing = admissions.get(content.admissionId) ?? departed;
      if (!isNewerAdmissionState(next, existing)) return false;
      if (
        !existing &&
        next.rosterRevision !== null &&
        session.compactedRosterRevisionFloor !== null &&
        next.rosterRevision <= session.compactedRosterRevisionFloor
      ) {
        return false;
      }
      if (next.present) {
        session.departedAdmissions.delete(content.admissionId);
        admissions.set(content.admissionId, next);
        session.admissionsByParticipant.set(participant, admissions);
      } else {
        admissions.delete(content.admissionId);
        if (admissions.size === 0) {
          session.admissionsByParticipant.delete(participant);
        }
        session.departedAdmissions.delete(content.admissionId);
        session.departedAdmissions.set(content.admissionId, next);
        while (session.departedAdmissions.size > 1_000) {
          const oldest = session.departedAdmissions.entries().next().value as
            | [string, AdmissionState]
            | undefined;
          if (!oldest) break;
          session.departedAdmissions.delete(oldest[0]);
          if (oldest[1].rosterRevision !== null) {
            session.compactedRosterRevisionFloor = Math.max(
              session.compactedRosterRevisionFloor ?? -1,
              oldest[1].rosterRevision,
            );
          }
        }
      }
    } else {
      const existing = session.legacyStateByParticipant.get(participant);
      if (!isNewerAdmissionState(next, existing)) return false;
      session.legacyStateByParticipant.set(participant, next);
    }
    if (content.rosterRevision !== null && isAfterLatestRosterEvent) {
      session.latestRosterRevision = content.rosterRevision;
      session.latestRosterCreatedAt = event.created_at;
      session.latestRosterEventId = event.id;
    }
    return true;
  }

  reconcileLiveness(
    generations: ReadonlyMap<string, LivenessGeneration>,
    previousGenerations: ReadonlyMap<string, LivenessGeneration> = new Map(),
  ): void {
    for (const [sessionId, session] of this.sessions) {
      const generation = generations.get(sessionId);
      if (generation === undefined) continue;
      const previous = previousGenerations.get(sessionId) ?? session.generation;
      if (
        previous !== null &&
        previous !== undefined &&
        previous !== generation
      ) {
        session.admissionsByParticipant.clear();
        session.legacyStateByParticipant.clear();
        session.departedAdmissions.clear();
        session.compactedRosterRevisionFloor = null;
      }
      session.generation = generation;
      session.generationIsAuthoritative = true;
    }
  }

  private compactEndedSessions(): void {
    if (this.sessions.size <= 1_000) return;
    for (const [sessionId, session] of this.sessions) {
      if (session.endState) this.sessions.delete(sessionId);
      if (this.sessions.size <= 1_000) break;
    }
  }

  snapshot(activeSessionIds?: ReadonlySet<string>): ReadonlySet<string> {
    this.compactEndedSessions();
    const participants = new Set<string>();
    for (const [sessionId, session] of this.sessions) {
      if (activeSessionIds && !activeSessionIds.has(sessionId)) continue;
      if (session.endState) continue;
      for (const participant of sessionParticipants(session)) {
        participants.add(participant);
      }
    }
    return participants;
  }
}

/** Apply a complete hydration result in lifecycle order to an existing tracker. */
export function applyHuddleLifecycleHistory(
  tracker: HuddlePresenceTracker,
  events: Iterable<RelayEvent>,
): void {
  for (const event of [...events].sort(compareHuddleLifecycleEvents)) {
    tracker.apply(event);
  }
}

/** Reconstruct everyone currently in an authenticated, visible huddle. */
export function reconstructHuddlePresence(
  events: Iterable<RelayEvent>,
  relaySelfPubkey: string | null | undefined,
): ReadonlySet<string> {
  const tracker = new HuddlePresenceTracker(relaySelfPubkey);
  applyHuddleLifecycleHistory(tracker, events);
  return tracker.snapshot();
}
