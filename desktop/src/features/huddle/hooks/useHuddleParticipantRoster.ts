import * as React from "react";

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
};

function normalizedPubkey(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  return normalized ? normalized : null;
}

function lifecycleChannelId(event: RelayEvent): string | null {
  try {
    const content = JSON.parse(event.content) as {
      ephemeral_channel_id?: unknown;
    };
    return typeof content.ephemeral_channel_id === "string"
      ? content.ephemeral_channel_id
      : null;
  } catch {
    return null;
  }
}

function lifecycleParticipant(event: RelayEvent): string | null {
  return normalizedPubkey(
    event.tags.find((tag) => tag[0] === "p")?.[1] ?? event.pubkey,
  );
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
}: ParticipantRosterOptions): string[] {
  const participants = new Set(
    fallbackParticipants
      .map(normalizedPubkey)
      .filter((pubkey): pubkey is string => pubkey !== null),
  );
  const sorted = [...events]
    .filter((event) => lifecycleChannelId(event) === ephemeralChannelId)
    // Equal-second lifecycle events must retain relay delivery order: kind/id
    // are not causal and can invert a leave followed by an immediate rejoin.
    .sort((left, right) => left.created_at - right.created_at);
  let ended = false;

  for (const event of sorted) {
    switch (event.kind) {
      case KIND_HUDDLE_STARTED: {
        ended = false;
        participants.clear();
        const creator = normalizedPubkey(event.pubkey);
        if (creator) participants.add(creator);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_JOINED: {
        if (ended) break;
        const participant = lifecycleParticipant(event);
        if (participant) participants.add(participant);
        break;
      }
      case KIND_HUDDLE_PARTICIPANT_LEFT: {
        if (ended) break;
        const participant = lifecycleParticipant(event);
        if (participant) participants.delete(participant);
        break;
      }
      case KIND_HUDDLE_ENDED:
        ended = true;
        participants.clear();
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
};

/** Subscribe the active desktop roster to the canonical signed lifecycle. */
export function useHuddleParticipantRoster({
  parentChannelId,
  ephemeralChannelId,
  fallbackParticipants,
  preservedParticipants = [],
}: UseHuddleParticipantRosterOptions): string[] {
  const sessionKey =
    parentChannelId && ephemeralChannelId
      ? `${parentChannelId}:${ephemeralChannelId}`
      : null;
  const [lifecycle, setLifecycle] = React.useState<{
    sessionKey: string;
    events: Map<string, RelayEvent>;
  } | null>(null);

  React.useEffect(() => {
    if (!sessionKey || !parentChannelId) return;

    let disposed = false;
    let cleanup: (() => void) | null = null;
    setLifecycle({ sessionKey, events: new Map() });

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
          return { sessionKey, events };
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

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [ephemeralChannelId, parentChannelId, sessionKey]);

  const events =
    lifecycle?.sessionKey === sessionKey ? lifecycle.events.values() : [];
  if (!ephemeralChannelId) {
    return reconstructHuddleParticipantRoster({
      ephemeralChannelId: "",
      events: [],
      fallbackParticipants,
      preservedParticipants,
    });
  }
  return reconstructHuddleParticipantRoster({
    ephemeralChannelId,
    events,
    fallbackParticipants,
    preservedParticipants,
  });
}
