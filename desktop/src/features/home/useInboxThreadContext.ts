import * as React from "react";

import { relayEventFromFeedItem } from "@/features/home/lib/inbox";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import { getEventById } from "@/shared/api/tauri";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import { HOME_MENTION_EVENT_KINDS } from "@/shared/constants/kinds";

type InboxThreadContextResult = {
  events: RelayEvent[];
  isLoading: boolean;
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

function isSameChannel(event: RelayEvent, channelId: string | null): boolean {
  if (!channelId) {
    return true;
  }
  return getChannelIdFromTags(event.tags) === channelId;
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
        const eventIds = new Set<string>([threadRootId]);
        if (selectedParentId) {
          eventIds.add(selectedParentId);
        }

        const ancestorEvents = await Promise.all(
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

        const descendantEvents =
          selectedChannelId && threadRootId
            ? await relayClient
                .fetchEvents({
                  "#e": [threadRootId],
                  "#h": [selectedChannelId],
                  kinds: [...HOME_MENTION_EVENT_KINDS],
                  limit: THREAD_CONTEXT_LIMIT,
                })
                .catch(() => [])
            : [];

        if (isCancelled) {
          return;
        }

        setFetchedEvents(
          dedupeEvents(
            [...ancestorEvents, ...descendantEvents].filter(
              (event): event is RelayEvent =>
                event !== null && isSameChannel(event, selectedChannelId),
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
      if (!isSameChannel(event, selectedChannelId)) {
        return false;
      }

      if (event.id === selectedEvent.id) {
        return true;
      }

      const thread = getThreadReference(event.tags);
      return (
        event.id === selectedThreadRootId ||
        event.id === selectedParentId ||
        thread.rootId === selectedThreadRootId ||
        thread.parentId === selectedThreadRootId ||
        thread.parentId === selectedEvent.id
      );
    });

    return dedupeEvents([selectedEvent, ...fetchedEvents, ...localContext]);
  }, [
    channelMessages,
    fetchedEvents,
    selectedChannelId,
    selectedEvent,
    selectedParentId,
    selectedThreadRootId,
  ]);

  return {
    events,
    isLoading,
  };
}
