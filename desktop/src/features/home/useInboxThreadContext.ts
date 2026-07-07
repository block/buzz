import * as React from "react";

import { isInboxThreadContextEvent } from "@/features/home/lib/inboxViewHelpers";
import { relayEventFromFeedItem } from "@/features/home/lib/inbox";
import { getThreadReference } from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters";
import { getEventById } from "@/shared/api/tauri";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import { HOME_MENTION_EVENT_KINDS } from "@/shared/constants/kinds";

type InboxThreadContextResult = {
  events: RelayEvent[];
  isLoading: boolean;
  /** kind:7 events referencing the context messages, fetched by `#e`. */
  reactionEvents: RelayEvent[];
  /** Re-fetch reaction events (e.g. after a toggle) without reloading context. */
  refreshReactions: () => Promise<void>;
};

const THREAD_CONTEXT_LIMIT = 100;

function dedupeEvents(events: RelayEvent[]): RelayEvent[] {
  const eventsById = new Map<string, RelayEvent>();
  for (const event of events) {
    eventsById.set(event.id, event);
  }
  return [...eventsById.values()].sort((a, b) => a.created_at - b.created_at);
}

function getThreadRootId(event: RelayEvent): string {
  const thread = getThreadReference(event.tags);
  return thread.rootId ?? thread.parentId ?? event.id;
}

export function useInboxThreadContext(
  item: FeedItem | null,
  channelMessages: RelayEvent[] | undefined,
): InboxThreadContextResult {
  const [fetchedEvents, setFetchedEvents] = React.useState<RelayEvent[]>([]);
  const [isLoading, setIsLoading] = React.useState(false);

  const selectedEvent = React.useMemo(
    () => (item ? relayEventFromFeedItem(item) : null),
    [item],
  );

  const selectedThreadRootId = selectedEvent
    ? getThreadRootId(selectedEvent)
    : null;
  const selectedParentId = selectedEvent
    ? getThreadReference(selectedEvent.tags).parentId
    : null;
  const selectedChannelId = item?.channelId ?? null;

  React.useEffect(() => {
    let isCancelled = false;

    if (!selectedEvent || !selectedThreadRootId) {
      setFetchedEvents([]);
      setIsLoading(false);
      return () => {
        isCancelled = true;
      };
    }

    async function loadContext() {
      const targetEvent = selectedEvent;
      const threadRootId = selectedThreadRootId;
      if (!targetEvent || !threadRootId) {
        return;
      }

      setIsLoading(true);

      try {
        const selection = {
          selectedChannelId,
          selectedEventId: targetEvent.id,
          selectedParentId,
          selectedThreadRootId: threadRootId,
        };
        const eventIds = new Set<string>([threadRootId]);
        if (selectedParentId) {
          eventIds.add(selectedParentId);
        }

        const ancestorEventsPromise = Promise.all(
          [...eventIds]
            .filter((eventId) => eventId !== targetEvent.id)
            .map(async (eventId) => {
              try {
                return await getEventById(eventId);
              } catch {
                return null;
              }
            }),
        );

        const descendantEventsPromise =
          selectedChannelId && threadRootId
            ? relayClient
                .fetchEvents({
                  "#e": [threadRootId],
                  "#h": [selectedChannelId],
                  kinds: [...HOME_MENTION_EVENT_KINDS],
                  limit: THREAD_CONTEXT_LIMIT,
                })
                .catch(() => [])
            : Promise.resolve([]);
        const [ancestorEvents, descendantEvents] = await Promise.all([
          ancestorEventsPromise,
          descendantEventsPromise,
        ]);

        if (isCancelled) {
          return;
        }

        setFetchedEvents(
          dedupeEvents(
            [...ancestorEvents, ...descendantEvents].filter(
              (event): event is RelayEvent =>
                event !== null && isInboxThreadContextEvent(event, selection),
            ),
          ),
        );
      } finally {
        if (!isCancelled) {
          setIsLoading(false);
        }
      }
    }

    void loadContext();

    return () => {
      isCancelled = true;
    };
  }, [
    selectedChannelId,
    selectedEvent,
    selectedParentId,
    selectedThreadRootId,
  ]);

  const events = React.useMemo(() => {
    if (!selectedEvent) {
      return [];
    }

    const localContext = (channelMessages ?? []).filter((event) => {
      return isInboxThreadContextEvent(event, {
        selectedChannelId,
        selectedEventId: selectedEvent.id,
        selectedParentId,
        selectedThreadRootId,
      });
    });

    const currentFetchedEvents = fetchedEvents.filter((event) =>
      isInboxThreadContextEvent(event, {
        selectedChannelId,
        selectedEventId: selectedEvent.id,
        selectedParentId,
        selectedThreadRootId,
      }),
    );

    return dedupeEvents([
      selectedEvent,
      ...currentFetchedEvents,
      ...localContext,
    ]);
  }, [
    channelMessages,
    fetchedEvents,
    selectedChannelId,
    selectedEvent,
    selectedParentId,
    selectedThreadRootId,
  ]);

  // Reactions carry only an `#e` reference, so the channel-window cache never
  // has them for thread replies — fetch them for the rendered context messages.
  const [reactionEvents, setReactionEvents] = React.useState<RelayEvent[]>([]);
  const contextEventIdsKey = React.useMemo(
    () =>
      events
        .map((event) => event.id)
        .sort()
        .join(","),
    [events],
  );

  const fetchReactions = React.useCallback(async (): Promise<
    RelayEvent[] | null
  > => {
    const eventIds = contextEventIdsKey ? contextEventIdsKey.split(",") : [];
    if (!selectedChannelId || eventIds.length === 0) {
      return [];
    }

    try {
      return await relayClient.fetchAuxEventsByReference(
        selectedChannelId,
        eventIds,
        buildChannelReactionAuxFilter,
      );
    } catch (error) {
      console.error(
        "Failed to hydrate reactions for Inbox context messages",
        selectedChannelId,
        error,
      );
      return null;
    }
  }, [contextEventIdsKey, selectedChannelId]);

  React.useEffect(() => {
    let isCancelled = false;

    void fetchReactions().then((fetched) => {
      if (!isCancelled && fetched !== null) {
        setReactionEvents(fetched);
      }
    });

    return () => {
      isCancelled = true;
    };
  }, [fetchReactions]);

  const refreshReactions = React.useCallback(async () => {
    const fetched = await fetchReactions();
    if (fetched !== null) {
      setReactionEvents(fetched);
    }
  }, [fetchReactions]);

  return {
    events,
    isLoading,
    reactionEvents,
    refreshReactions,
  };
}
