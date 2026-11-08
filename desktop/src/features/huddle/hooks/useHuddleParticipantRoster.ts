import * as React from "react";

import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

type ParticipantRosterOptions = {
  ephemeralChannelId: string;
  events: Iterable<RelayEvent>;
  fallbackParticipants?: readonly string[];
  preservedParticipants?: readonly (string | null | undefined)[];
  relaySelfPubkey?: string | null;
  huddleThreadEventId?: string | null;
  canonicalStartEvent?: RelayEvent | null;
};

function normalizedPubkey(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  return normalized ? normalized : null;
}

function lifecycleContent(event: RelayEvent): {
  ephemeralChannelId: string | null;
  rosterRevision: number | null;
  admissionId: string | null;
} {
  try {
    const content = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
      roster_revision?: unknown;
      admission_id?: unknown;
    };
    return {
      ephemeralChannelId:
        typeof content.ephemeral_channel_id === "string"
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
    };
  } catch {
    return {
      ephemeralChannelId: null,
      rosterRevision: null,
      admissionId: null,
    };
  }
}

function lifecycleChannelId(event: RelayEvent): string | null {
  return lifecycleContent(event).ephemeralChannelId;
}

function lifecyclePhase(kind: number): number {
  if (kind === KIND_HUDDLE_STARTED) return 0;
  if (kind === KIND_HUDDLE_ENDED) return 2;
  return 1;
}

function compareLifecycleEvents(left: RelayEvent, right: RelayEvent): number {
  const timestampOrder = left.created_at - right.created_at;
  if (timestampOrder !== 0) return timestampOrder;

  const phaseOrder = lifecyclePhase(left.kind) - lifecyclePhase(right.kind);
  if (phaseOrder !== 0) return phaseOrder;

  const leftParticipant = lifecycleParticipant(left);
  const rightParticipant = lifecycleParticipant(right);
  const sameParticipant =
    leftParticipant !== null && leftParticipant === rightParticipant;
  const leftContent = lifecycleContent(left);
  const rightContent = lifecycleContent(right);
  if (
    sameParticipant &&
    leftContent.admissionId !== null &&
    leftContent.admissionId === rightContent.admissionId &&
    left.kind !== right.kind
  ) {
    return left.kind === KIND_HUDDLE_PARTICIPANT_JOINED ? -1 : 1;
  }
  const leftRevision = leftContent.rosterRevision;
  const rightRevision = rightContent.rosterRevision;
  if (sameParticipant && leftRevision !== rightRevision) {
    if (leftRevision === null) {
      return left.kind === KIND_HUDDLE_PARTICIPANT_LEFT ? -1 : 1;
    }
    if (rightRevision === null) {
      return right.kind === KIND_HUDDLE_PARTICIPANT_LEFT ? 1 : -1;
    }
    return leftRevision - rightRevision;
  }
  if (
    leftRevision !== null &&
    rightRevision !== null &&
    leftRevision !== rightRevision
  ) {
    return leftRevision - rightRevision;
  }

  return left.kind - right.kind || left.id.localeCompare(right.id);
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return normalizedPubkey(
    event.tags.find((tag) => tag[0] === "p")?.[1] ?? event.pubkey,
  );
}

function authenticatedLifecycleEvents({
  events,
  relaySelfPubkey,
  huddleThreadEventId,
  canonicalStartEvent,
}: Pick<
  ParticipantRosterOptions,
  "events" | "relaySelfPubkey" | "huddleThreadEventId" | "canonicalStartEvent"
>): RelayEvent[] {
  const relaySelf = normalizedPubkey(relaySelfPubkey);
  const lifecycle = [...events];
  if (
    canonicalStartEvent &&
    !lifecycle.some((event) => event.id === canonicalStartEvent.id)
  ) {
    lifecycle.push(canonicalStartEvent);
  }
  const started = huddleThreadEventId
    ? (lifecycle.find(
        (event) =>
          event.kind === KIND_HUDDLE_STARTED &&
          event.id === huddleThreadEventId,
      ) ?? null)
    : null;
  const creator = started ? normalizedPubkey(started.pubkey) : null;

  return lifecycle.filter((event) => {
    if (
      event.kind === KIND_HUDDLE_PARTICIPANT_JOINED ||
      event.kind === KIND_HUDDLE_PARTICIPANT_LEFT
    ) {
      return relaySelf !== null && normalizedPubkey(event.pubkey) === relaySelf;
    }
    if (event.kind === KIND_HUDDLE_STARTED) {
      return started !== null && event.id === started.id;
    }
    if (event.kind === KIND_HUDDLE_ENDED) {
      const signer = normalizedPubkey(event.pubkey);
      return signer !== null && (signer === creator || signer === relaySelf);
    }
    return false;
  });
}

/**
 * Reconstruct the live media roster from relay-signed Huddle lifecycle events.
 *
 * Backing-channel membership remains the access-control fallback. Once the
 * lifecycle stream is present, joins and disconnect-driven leaves are applied
 * in causal order so clients do not have to wait for membership polling.
 */
export function reconstructHuddleParticipantRoster({
  ephemeralChannelId,
  events,
  fallbackParticipants = [],
  preservedParticipants = [],
  relaySelfPubkey = null,
  huddleThreadEventId = null,
  canonicalStartEvent = null,
}: ParticipantRosterOptions): string[] {
  const lifecycleEvents = [...events];
  const canonicalStartOutsideWindow =
    canonicalStartEvent !== null &&
    !lifecycleEvents.some((event) => event.id === canonicalStartEvent.id);
  const participants = new Set(
    fallbackParticipants
      .map(normalizedPubkey)
      .filter((pubkey): pubkey is string => pubkey !== null),
  );
  const sorted = authenticatedLifecycleEvents({
    events: lifecycleEvents,
    relaySelfPubkey,
    huddleThreadEventId,
    canonicalStartEvent,
  })
    .filter((event) => lifecycleChannelId(event) === ephemeralChannelId)
    // Nostr timestamps have second precision and relay history arrives newest
    // first. Session phases break start/end ties; owner roster revisions order
    // same-second join/leave mutations without relying on delivery order.
    .sort(compareLifecycleEvents);
  let ended = false;
  const admissionIdsByParticipant = new Map<string, Set<string>>();

  for (const event of sorted) {
    switch (event.kind) {
      case KIND_HUDDLE_STARTED: {
        ended = false;
        // A canonical start fetched separately means the bounded lifecycle
        // history no longer contains the whole session. Preserve the live
        // membership fallback instead of erasing long-lived participants whose
        // join fell outside that window.
        if (!canonicalStartOutsideWindow) participants.clear();
        admissionIdsByParticipant.clear();
        const creator = normalizedPubkey(event.pubkey);
        if (creator) participants.add(creator);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        if (ended) break;
        const participant = lifecycleParticipant(event);
        if (participant) {
          const admissionId = lifecycleContent(event).admissionId;
          if (admissionId) {
            const admissionIds =
              admissionIdsByParticipant.get(participant) ?? new Set<string>();
            admissionIds.add(admissionId);
            admissionIdsByParticipant.set(participant, admissionIds);
          }
          participants.add(participant);
        }
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        if (ended) break;
        const participant = lifecycleParticipant(event);
        if (participant) {
          const admissionId = lifecycleContent(event).admissionId;
          const admissionIds = admissionIdsByParticipant.get(participant);
          if (admissionId && admissionIds) {
            admissionIds.delete(admissionId);
            if (admissionIds.size > 0) break;
          }
          admissionIdsByParticipant.delete(participant);
          participants.delete(participant);
        }
        break;
      }
      case KIND_HUDDLE_ENDED:
        ended = true;
        participants.clear();
        admissionIdsByParticipant.clear();
        break;
    }
  }

  if (!ended) {
    for (const value of preservedParticipants) {
      const participant = normalizedPubkey(value);
      if (participant) participants.add(participant);
    }
  }

  return [...participants];
}

type UseHuddleParticipantRosterOptions = {
  parentChannelId: string | null;
  ephemeralChannelId: string | null;
  fallbackParticipants: readonly string[];
  preservedParticipants?: readonly (string | null | undefined)[];
  huddleThreadEventId?: string | null;
};

/** Subscribe the active desktop roster to the canonical signed lifecycle. */
export function useHuddleParticipantRoster({
  parentChannelId,
  ephemeralChannelId,
  fallbackParticipants,
  preservedParticipants = [],
  huddleThreadEventId = null,
}: UseHuddleParticipantRosterOptions): string[] {
  const relaySelfPubkey = useRelaySelfQuery(Boolean(parentChannelId)).data;
  const sessionKey =
    parentChannelId && ephemeralChannelId
      ? `${parentChannelId}:${ephemeralChannelId}`
      : null;
  const [lifecycle, setLifecycle] = React.useState<{
    sessionKey: string;
    events: Map<string, RelayEvent>;
    canonicalStartEvent: RelayEvent | null;
  } | null>(null);

  React.useEffect(() => {
    if (!sessionKey || !parentChannelId) return;

    let disposed = false;
    let cleanup: (() => void) | null = null;
    setLifecycle({
      sessionKey,
      events: new Map(),
      canonicalStartEvent: null,
    });

    void relayClient
      .subscribeToHuddleEvents(parentChannelId, (event) => {
        if (disposed || lifecycleChannelId(event) !== ephemeralChannelId)
          return;
        setLifecycle((current) => {
          const events =
            current?.sessionKey === sessionKey
              ? new Map(current.events)
              : new Map<string, RelayEvent>();
          if (events.has(event.id)) return current;
          events.set(event.id, event);
          return {
            sessionKey,
            events,
            canonicalStartEvent:
              current?.sessionKey === sessionKey
                ? current.canonicalStartEvent
                : null,
          };
        });
      })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error(
          "[huddle] Participant lifecycle subscription failed:",
          error,
        );
      });

    if (huddleThreadEventId) {
      void relayClient
        .fetchEvents({
          ids: [huddleThreadEventId],
          kinds: [KIND_HUDDLE_STARTED],
          limit: 1,
        })
        .then(([canonicalStart]) => {
          if (!canonicalStart || disposed) return;
          setLifecycle((current) => {
            const events =
              current?.sessionKey === sessionKey
                ? new Map(current.events)
                : new Map<string, RelayEvent>();
            return { sessionKey, events, canonicalStartEvent: canonicalStart };
          });
        })
        .catch((error) => {
          console.error("[huddle] Canonical start lookup failed:", error);
        });
    }

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [ephemeralChannelId, huddleThreadEventId, parentChannelId, sessionKey]);

  const events =
    lifecycle?.sessionKey === sessionKey ? lifecycle.events.values() : [];
  if (!ephemeralChannelId) {
    return reconstructHuddleParticipantRoster({
      ephemeralChannelId: "",
      events: [],
      fallbackParticipants,
      preservedParticipants,
      relaySelfPubkey,
      huddleThreadEventId,
    });
  }
  return reconstructHuddleParticipantRoster({
    ephemeralChannelId,
    events,
    fallbackParticipants,
    preservedParticipants,
    relaySelfPubkey,
    huddleThreadEventId,
    canonicalStartEvent:
      lifecycle?.sessionKey === sessionKey
        ? lifecycle.canonicalStartEvent
        : null,
  });
}
