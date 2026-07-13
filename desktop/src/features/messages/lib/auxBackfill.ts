import type { QueryClient } from "@tanstack/react-query";

import {
  channelMessagesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelStructuralAuxFilter } from "@/shared/api/relayChannelFilters";
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

/**
 * Structural aux closure (edits/deletions + deletions of those aux events)
 * for an explicit set of message ids, returned to the caller instead of being
 * merged into the channel cache. The thread-replies fetch uses this: the
 * server thread-subtree query resolves deletions itself but returns content
 * kinds only, so a reply's kind:40003 edit never rides along — without this
 * backfill a refetch (thread reopen, channel switch) renders the original,
 * un-edited text.
 */
export type StructuralAuxFetchDeps = {
  fetchAuxEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
  fetchAuxDeletionEventsForAuxEvents: (
    channelId: string,
    auxEventIds: string[],
  ) => Promise<RelayEvent[]>;
};

const defaultStructuralAuxDeps: StructuralAuxFetchDeps = {
  fetchAuxEventsForMessages: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelStructuralAuxFilter,
    ),
  fetchAuxDeletionEventsForAuxEvents: (channelId, auxEventIds) =>
    relayClient.fetchAuxDeletionEventsForAuxEvents(channelId, auxEventIds),
};

export async function fetchStructuralAuxForMessages(
  channelId: string,
  messageIds: string[],
  deps: StructuralAuxFetchDeps = defaultStructuralAuxDeps,
): Promise<RelayEvent[]> {
  if (messageIds.length === 0) {
    return [];
  }
  const auxEvents = await deps.fetchAuxEventsForMessages(channelId, messageIds);
  return mergeAuxEventsWithDeletionBackfill({
    channelId,
    cachedEvents: [],
    fetchedAuxEvents: auxEvents,
    fetchAuxEventsForMessages: deps.fetchAuxDeletionEventsForAuxEvents,
  });
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
  if (messageIds.length === 0) {
    return;
  }

  try {
    const cacheKey = channelMessagesKey(channelId);
    const cachedEvents = queryClient.getQueryData<RelayEvent[]>(cacheKey) ?? [];
    const auxEvents = await relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelStructuralAuxFilter,
    );
    const mergedAuxEvents = await mergeAuxEventsWithDeletionBackfill({
      channelId,
      cachedEvents,
      fetchedAuxEvents: auxEvents,
      fetchAuxEventsForMessages: (...args) =>
        relayClient.fetchAuxDeletionEventsForAuxEvents(...args),
    });
    if (mergedAuxEvents.length === 0) {
      return;
    }

    queryClient.setQueryData<RelayEvent[]>(cacheKey, (current = []) =>
      sortMessages([...current, ...mergedAuxEvents]),
    );
  } catch (error) {
    console.error(
      "Failed to backfill auxiliary events for channel",
      channelId,
      error,
    );
  }
}
