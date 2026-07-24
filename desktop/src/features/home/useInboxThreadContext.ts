import * as React from "react";

import { isInboxThreadContextEvent } from "@/features/home/lib/inboxViewHelpers";
import { relayEventFromFeedItem } from "@/features/home/lib/inbox";
import { getThreadReference } from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters";
import { getEventById } from "@/shared/api/tauri";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_TIMELINE_CONTENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
} from "@/shared/constants/kinds";

type InboxThreadContextResult = {
  events: RelayEvent[];
  hasLoadError: boolean;
  isLoading: boolean;
  /** kind:7 events referencing the context messages, fetched by `#e`. */
  reactionEvents: RelayEvent[];
  /** Re-fetch reaction events (e.g. after a toggle) without reloading context. */
  refreshReactions: () => Promise<void>;
};

const THREAD_CONTEXT_LIMIT = 100;
const MAX_ANCESTOR_HOPS = 50;
const CHANNEL_CONTEXT_EVENT_KINDS = new Set<number>(
  CHANNEL_TIMELINE_CONTENT_KINDS,
);

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
  options: {
    fullChannel?: boolean;
    hasChannelLoadError?: boolean;
    isChannelLoading?: boolean;
  } = {},
): InboxThreadContextResult {
  const [fetchedEvents, setFetchedEvents] = React.useState<RelayEvent[]>([]);
  const [hasLoadError, setHasLoadError] = React.useState(false);
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
  const fullChannel = options.fullChannel === true;

  React.useEffect(() => {
    let isCancelled = false;

    if (fullChannel || !selectedEvent || !selectedThreadRootId) {
      setFetchedEvents([]);
      setHasLoadError(false);
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
      setHasLoadError(false);

      try {
        const selection = {
          selectedChannelId,
          selectedEventId: targetEvent.id,
          selectedParentId,
          selectedThreadRootId: threadRootId,
        };
        const ancestorEventsPromise = (async () => {
          const eventsById = new Map<string, RelayEvent>();
          let failed = false;

          const fetchEvent = async (eventId: string) => {
            if (eventId === targetEvent.id || eventsById.has(eventId)) {
              return eventsById.get(eventId) ?? targetEvent;
            }

            try {
              const event = await getEventById(eventId);
              eventsById.set(event.id, event);
              return event;
            } catch {
              failed = true;
              return null;
            }
          };

          if (threadRootId !== targetEvent.id) {
            await fetchEvent(threadRootId);
          }

          let ancestorId = selectedParentId;
          const seen = new Set<string>([targetEvent.id]);
          let hops = 0;
          while (
            ancestorId &&
            !seen.has(ancestorId) &&
            hops < MAX_ANCESTOR_HOPS
          ) {
            seen.add(ancestorId);
            const ancestor = await fetchEvent(ancestorId);
            if (!ancestor || ancestorId === threadRootId) {
              break;
            }
            ancestorId = getThreadReference(ancestor.tags).parentId;
            hops += 1;
          }

          return { events: [...eventsById.values()], failed };
        })();

        const descendantEventsPromise =
          selectedChannelId && threadRootId
            ? relayClient
                .fetchEvents({
                  "#e": [threadRootId],
                  "#h": [selectedChannelId],
                  kinds: [...HOME_MENTION_EVENT_KINDS],
                  limit: THREAD_CONTEXT_LIMIT,
                })
                .then((events) => ({ events, failed: false }))
                .catch((error) => {
                  console.error(
                    "Failed to hydrate Activity thread context",
                    selectedChannelId,
                    threadRootId,
                    error,
                  );
                  return { events: [] as RelayEvent[], failed: true };
                })
            : Promise.resolve({ events: [] as RelayEvent[], failed: false });
        const [ancestorResult, descendantResult] = await Promise.all([
          ancestorEventsPromise,
          descendantEventsPromise,
        ]);

        if (isCancelled) {
          return;
        }

        setHasLoadError(ancestorResult.failed || descendantResult.failed);
        setFetchedEvents(
          dedupeEvents(
            [...ancestorResult.events, ...descendantResult.events].filter(
              (event): event is RelayEvent =>
                event !== null && isInboxThreadContextEvent(event, selection),
            ),
          ),
        );
      } catch (error) {
        if (!isCancelled) {
          console.error("Failed to load Activity message context", error);
          setHasLoadError(true);
        }
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
    fullChannel,
  ]);

  const events = React.useMemo(() => {
    if (!selectedEvent) {
      return [];
    }

    if (fullChannel) {
      return dedupeEvents([
        selectedEvent,
        ...(channelMessages ?? []).filter((event) =>
          CHANNEL_CONTEXT_EVENT_KINDS.has(event.kind),
        ),
      ]);
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
    fullChannel,
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
    hasLoadError: fullChannel
      ? options.hasChannelLoadError === true
      : hasLoadError,
    isLoading: fullChannel ? options.isChannelLoading === true : isLoading,
    reactionEvents,
    refreshReactions,
  };
}
