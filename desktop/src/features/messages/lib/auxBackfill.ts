import type { QueryClient } from "@tanstack/react-query";

import {
  channelMessagesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import {
  buildChannelAuxDeletionFilter,
  buildChannelReactionAuxFilter,
  buildChannelStructuralAuxFilter,
} from "@/shared/api/relayChannelFilters";
import type { RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_TIMELINE_CONTENT_KINDS,
  KIND_REACTION,
  KIND_STREAM_MESSAGE_EDIT,
} from "@/shared/constants/kinds";

const TIMELINE_CONTENT_KINDS: ReadonlySet<number> = new Set(
  CHANNEL_TIMELINE_CONTENT_KINDS,
);

/**
 * Extract the ids of the visible content messages from a freshly-fetched
 * history window. Auxiliary events (reactions, edits, deletions) are then
 * backfilled by `#e` reference over exactly these ids. Pure so it can be
 * unit-tested without a relay or query client.
 */
export function collectMessageIdsForAuxBackfill(
  historyEvents: RelayEvent[],
): string[] {
  return historyEvents
    .filter((event) => TIMELINE_CONTENT_KINDS.has(event.kind))
    .map((event) => event.id);
}

export function collectAuxEventIdsForDeletionBackfill(
  auxEvents: RelayEvent[],
): string[] {
  return auxEvents
    .filter(
      (event) =>
        event.kind === KIND_REACTION || event.kind === KIND_STREAM_MESSAGE_EDIT,
    )
    .map((event) => event.id);
}

export async function mergeAuxEventsWithDeletionBackfill(input: {
  channelId: string;
  cachedEvents: RelayEvent[];
  fetchedAuxEvents: RelayEvent[];
  fetchAuxEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
}): Promise<RelayEvent[]> {
  const auxEventIds = [
    ...new Set([
      ...collectAuxEventIdsForDeletionBackfill(input.cachedEvents),
      ...collectAuxEventIdsForDeletionBackfill(input.fetchedAuxEvents),
    ]),
  ];
  const auxDeletionEvents =
    auxEventIds.length > 0
      ? await input.fetchAuxEventsForMessages(input.channelId, auxEventIds)
      : [];

  return [...input.fetchedAuxEvents, ...auxDeletionEvents];
}

export interface AuxBackfillDeps {
  fetchReactionAuxEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
  fetchStructuralAuxEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
  fetchAuxDeletionEventsForAuxEvents: (
    channelId: string,
    auxEventIds: string[],
  ) => Promise<RelayEvent[]>;
}

const defaultAuxBackfillDeps: AuxBackfillDeps = {
  fetchReactionAuxEventsForMessages: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelReactionAuxFilter,
    ),
  fetchStructuralAuxEventsForMessages: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelStructuralAuxFilter,
    ),
  fetchAuxDeletionEventsForAuxEvents: (channelId, auxEventIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      auxEventIds,
      buildChannelAuxDeletionFilter,
    ),
};

/**
 * After a content-kinds-only history fetch, pull the auxiliary events
 * (reactions, edits, deletions) that reference the loaded messages — keyed by
 * `#e` over their ids, not by a time window — and merge them into the same
 * channel cache.
 *
 * History fetches request content kinds only so the `limit` budget buys
 * visible message depth (a reaction-heavy 200-event window was only ~136
 * messages). The cost is that an edit/deletion for a visible message can fall
 * outside any fetched time window — so aux must be pulled by reference, or a
 * visible message renders stale (un-edited / not-deleted).
 *
 * Reactions and the structural overlay (edits/deletions) are fetched and
 * committed *independently*: reactions ride their own fast kind:7 REQ and land
 * first, so the slow kind:5 deletion scan — which on a busy workspace queued
 * past the history-load timeout — can no longer strand them. Each half has its
 * own try/catch; a failure in one is logged but never blanks the other.
 */
export async function backfillAuxForMessages(
  queryClient: QueryClient,
  channelId: string,
  historyEvents: RelayEvent[],
  deps: AuxBackfillDeps = defaultAuxBackfillDeps,
): Promise<void> {
  const messageIds = collectMessageIdsForAuxBackfill(historyEvents);
  if (messageIds.length === 0) {
    return;
  }

  const cacheKey = channelMessagesKey(channelId);
  const commit = (events: RelayEvent[]) => {
    if (events.length === 0) {
      return;
    }
    queryClient.setQueryData<RelayEvent[]>(cacheKey, (current = []) =>
      sortMessages([...current, ...events]),
    );
  };

  // Reactions first, on their own fast REQ — the half that actually shows up
  // as data loss when it's dropped. Commit as soon as they land.
  try {
    const reactionEvents = await deps.fetchReactionAuxEventsForMessages(
      channelId,
      messageIds,
    );
    commit(reactionEvents);
  } catch (error) {
    console.error("Failed to backfill reactions for channel", channelId, error);
  }

  // Structural overlay (edits/deletions) second, isolated. A failure here only
  // leaves a message un-edited / not-deleted until the next backfill — never
  // blanks the reactions already committed above.
  try {
    const cachedEvents = queryClient.getQueryData<RelayEvent[]>(cacheKey) ?? [];
    const structuralEvents = await deps.fetchStructuralAuxEventsForMessages(
      channelId,
      messageIds,
    );
    const mergedAuxEvents = await mergeAuxEventsWithDeletionBackfill({
      channelId,
      cachedEvents,
      fetchedAuxEvents: structuralEvents,
      fetchAuxEventsForMessages: deps.fetchAuxDeletionEventsForAuxEvents,
    });
    commit(mergedAuxEvents);
  } catch (error) {
    console.error(
      "Failed to backfill auxiliary events for channel",
      channelId,
      error,
    );
  }
}
