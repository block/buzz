import type { QueryClient } from "@tanstack/react-query";

import { getThreadReference } from "@/features/messages/lib/threading";
import {
  channelMessagesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import {
  buildChannelAgentConversationMarkerFilter,
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

function isTimelineContentEvent(event: RelayEvent) {
  return TIMELINE_CONTENT_KINDS.has(event.kind);
}

/**
 * Extract the ids of the visible content messages from a freshly-fetched
 * history window. Auxiliary events (reactions, edits, deletions) are then
 * backfilled by `#e` reference over exactly these ids. Pure so it can be
 * unit-tested without a relay or query client.
 */
export function collectMessageIdsForAuxBackfill(
  historyEvents: RelayEvent[],
): string[] {
  return historyEvents.filter(isTimelineContentEvent).map((event) => event.id);
}

export function collectAgentConversationMarkerReferenceIds(
  historyEvents: RelayEvent[],
): string[] {
  const referenceIds = new Set<string>();

  for (const event of historyEvents) {
    if (!isTimelineContentEvent(event)) {
      continue;
    }

    referenceIds.add(event.id);
    const { parentId, rootId } = getThreadReference(event.tags);
    if (rootId) {
      referenceIds.add(rootId);
    }
    if (parentId) {
      referenceIds.add(parentId);
    }
  }

  return [...referenceIds];
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

/**
 * After a content-kinds-only history fetch, pull structural auxiliary events
 * (edits/deletions) that reference the loaded messages — keyed by `#e` over
 * their ids, not by a time window — and merge them into the same channel cache.
 * Reactions are hydrated separately for the rows the GUI renders.
 *
 * History fetches request content kinds only so the `limit` budget buys
 * visible message depth. The cost is that an edit/deletion for a visible
 * message can fall outside any fetched time window — so structural aux must be
 * pulled by reference, or a visible message renders stale (un-edited /
 * not-deleted).
 *
 * Best-effort: failures are logged but never reject, so a flaky overlay fetch
 * can't blank the freshly-loaded messages.
 */
export async function backfillAuxForMessages(
  queryClient: QueryClient,
  channelId: string,
  historyEvents: RelayEvent[],
): Promise<void> {
  const messageIds = collectMessageIdsForAuxBackfill(historyEvents);
  const markerReferenceIds =
    collectAgentConversationMarkerReferenceIds(historyEvents);
  if (messageIds.length === 0 && markerReferenceIds.length === 0) {
    return;
  }

  try {
    const cacheKey = channelMessagesKey(channelId);
    const cachedEvents = queryClient.getQueryData<RelayEvent[]>(cacheKey) ?? [];
    const [auxEvents, markerEvents] = await Promise.all([
      messageIds.length > 0
        ? relayClient.fetchAuxEventsByReference(
            channelId,
            messageIds,
            buildChannelStructuralAuxFilter,
          )
        : Promise.resolve([]),
      markerReferenceIds.length > 0
        ? relayClient.fetchAuxEventsByReference(
            channelId,
            markerReferenceIds,
            buildChannelAgentConversationMarkerFilter,
          )
        : Promise.resolve([]),
    ]);
    const mergedAuxEvents = await mergeAuxEventsWithDeletionBackfill({
      channelId,
      cachedEvents,
      fetchedAuxEvents: auxEvents,
      fetchAuxEventsForMessages: (...args) =>
        relayClient.fetchAuxDeletionEventsForAuxEvents(...args),
    });
    const eventsToMerge = [...markerEvents, ...mergedAuxEvents];
    if (eventsToMerge.length === 0) {
      return;
    }

    queryClient.setQueryData<RelayEvent[]>(cacheKey, (current = []) =>
      sortMessages([...current, ...eventsToMerge]),
    );
  } catch (error) {
    console.error(
      "Failed to backfill timeline reference events for channel",
      channelId,
      error,
    );
  }
}
