import * as React from "react";

import { isInboxThreadContextEvent } from "@/features/home/lib/inboxViewHelpers";
import { relayEventFromFeedItem } from "@/features/home/lib/inbox";
import { fetchStructuralAuxForMessages } from "@/features/messages/lib/auxBackfill";
import { getThreadReference } from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters";
import { getEventById } from "@/shared/api/tauri";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_TIMELINE_CONTENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
} from "@/shared/constants/kinds";

type InboxThreadContextResult = {
  events: RelayEvent[];
  hasLoadError: boolean;
  isLoading: boolean;
  /** Edits/deletions referencing context messages, fetched by `#e`. */
  structuralEvents: RelayEvent[];
  /** Re-fetch structural events after an Inbox edit is published. */
  refreshStructuralEvents: () => Promise<void>;
  /** kind:7 events referencing the context messages, fetched by `#e`. */
  reactionEvents: RelayEvent[];
  /** Re-fetch reaction events (e.g. after a toggle) without reloading context. */
  refreshReactions: () => Promise<void>;
};

const THREAD_CONTEXT_LIMIT = 100;
const THREAD_CONTEXT_TIMEOUT_MS = 15_000;
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

function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timeout = window.setTimeout(
      () => reject(new Error(message)),
      timeoutMs,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timeout);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timeout);
        reject(error);
      },
    );
  });
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

  // Feed polling replaces FeedItem objects even when the selected event has
  // not changed. Depend on stable event content instead of the object
  // reference so a refresh cannot continuously restart context hydration.
  const selectedEventKey = item
    ? JSON.stringify([
        item.id,
        item.kind,
        item.pubkey,
        item.createdAt,
        item.content,
        item.tags,
      ])
    : "";
  const selectedEventCacheRef = React.useRef<{
    key: string;
    event: RelayEvent | null;
  }>({ key: "", event: null });
  if (selectedEventCacheRef.current.key !== selectedEventKey) {
    selectedEventCacheRef.current = {
      key: selectedEventKey,
      event: item ? relayEventFromFeedItem(item) : null,
    };
  }
  const selectedEvent = selectedEventCacheRef.current.event;

  const selectedThreadRootId = selectedEvent
    ? getThreadRootId(selectedEvent)
    : null;
  const selectedParentId = selectedEvent
    ? getThreadReference(selectedEvent.tags).parentId
    : null;
  const selectedChannelId = item?.channelId ?? null;
  const fullChannel = options.fullChannel === true;
  const relayConnectionState = useRelayConnection();

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

    // Wait for the authenticated relay before starting cold-context reads.
    // Keeping the existing events here prevents a reconnect from blanking an
    // already-rendered thread; the `connected` transition reruns this effect.
    if (relayConnectionState !== "connected") {
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
                    "Failed to hydrate Inbox thread context",
                    selectedChannelId,
                    threadRootId,
                    error,
                  );
                  return { events: [] as RelayEvent[], failed: true };
                })
            : Promise.resolve({ events: [] as RelayEvent[], failed: false });
        const [ancestorResult, descendantResult] = await withTimeout(
          Promise.all([ancestorEventsPromise, descendantEventsPromise]),
          THREAD_CONTEXT_TIMEOUT_MS,
          "Timed out while loading surrounding thread context.",
        );

        if (isCancelled) {
          return;
        }

        // A context request can begin during login or a reconnect. Do not
        // turn that transient failure into a persistent red banner; the
        // connection-state dependency reruns hydration after AUTH succeeds.
        setHasLoadError(
          relayConnectionState === "connected" &&
            (ancestorResult.failed || descendantResult.failed),
        );
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
          console.error("Failed to load Inbox message context", error);
          setHasLoadError(relayConnectionState === "connected");
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
    relayConnectionState,
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

  // Auxiliary events carry only an `#e` reference, so they may be absent from
  // both the selected feed item and the channel-window cache. Hydrate them by
  // the context message ids so cold Inbox items receive edits, deletions, and
  // reactions without requiring the full channel timeline to be open.
  const contextEventIdsKey = React.useMemo(
    () =>
      events
        .map((event) => event.id)
        .sort()
        .join(","),
    [events],
  );
  const [structuralEvents, setStructuralEvents] = React.useState<RelayEvent[]>(
    [],
  );

  const fetchStructuralEvents = React.useCallback(async (): Promise<
    RelayEvent[] | null
  > => {
    const eventIds = contextEventIdsKey ? contextEventIdsKey.split(",") : [];
    if (!selectedChannelId || eventIds.length === 0) {
      return [];
    }

    try {
      // Two hops, not one. A deletion can target an edit event rather than the
      // original message, and `formatTimelineMessages` drops an edit only when
      // the edit's own id is in the deletion set. A one-hop fetch therefore
      // re-applies retracted content on a cold Inbox open. The channel and
      // thread paths already resolve this closure with the same helper.
      return await fetchStructuralAuxForMessages(selectedChannelId, eventIds);
    } catch (error) {
      console.error(
        "Failed to hydrate structural events for Inbox context messages",
        selectedChannelId,
        error,
      );
      return null;
    }
  }, [contextEventIdsKey, selectedChannelId]);

  React.useEffect(() => {
    let isCancelled = false;
    setStructuralEvents([]);

    void fetchStructuralEvents().then((fetched) => {
      if (!isCancelled && fetched !== null) {
        setStructuralEvents(fetched);
      }
    });

    return () => {
      isCancelled = true;
    };
  }, [fetchStructuralEvents]);

  const refreshStructuralEvents = React.useCallback(async () => {
    const fetched = await fetchStructuralEvents();
    if (fetched !== null) {
      setStructuralEvents(fetched);
    }
  }, [fetchStructuralEvents]);

  const [reactionEvents, setReactionEvents] = React.useState<RelayEvent[]>([]);

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
    setReactionEvents([]);

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
    structuralEvents,
    refreshStructuralEvents,
    reactionEvents,
    refreshReactions,
  };
}
