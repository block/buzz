import { sortEvents } from "@/shared/api/relayClientShared";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

export type EventFrontier = {
  createdAt: number;
  eventId: string;
};

export const FRONTIER_BRIDGE_PAGE_LIMIT = 500;

export function compareEventToFrontier(
  event: RelayEvent,
  frontier: EventFrontier,
) {
  if (event.created_at !== frontier.createdAt) {
    return event.created_at - frontier.createdAt;
  }
  return event.id.localeCompare(frontier.eventId);
}

export function isEventAfterFrontier(
  event: RelayEvent,
  frontier: EventFrontier,
) {
  return compareEventToFrontier(event, frontier) > 0;
}

export function shouldApplyBridgeEvent({
  event,
  frontier,
  knownEventIds,
}: {
  event: RelayEvent;
  frontier: EventFrontier;
  knownEventIds: ReadonlySet<string>;
}) {
  if (event.created_at > frontier.createdAt) {
    return true;
  }

  // NIP-01 `since` is second-granularity. Keep unknown events in the frontier
  // second even if their id sorts before the frontier id; they may have landed
  // in the history→live race and a strict `(created_at,id) > frontier` filter
  // would silently drop them. Known ids are harmless duplicates.
  return (
    event.created_at === frontier.createdAt && !knownEventIds.has(event.id)
  );
}

export function newestEventFrontier(
  events: RelayEvent[],
): EventFrontier | null {
  let frontier: EventFrontier | null = null;

  for (const event of events) {
    if (!frontier || isEventAfterFrontier(event, frontier)) {
      frontier = {
        createdAt: event.created_at,
        eventId: event.id,
      };
    }
  }

  return frontier;
}

export function buildFrontierBridgeFilter({
  frontier,
  limit = FRONTIER_BRIDGE_PAGE_LIMIT,
  targetFilter,
}: {
  frontier: EventFrontier;
  limit?: number;
  targetFilter: RelaySubscriptionFilter;
}): RelaySubscriptionFilter {
  return {
    ...targetFilter,
    limit,
    since:
      targetFilter.since === undefined
        ? frontier.createdAt
        : Math.max(targetFilter.since, frontier.createdAt),
  };
}

/**
 * Fetch and apply the explicit gap bridge after a live subscription is ready.
 *
 * The relay's NIP-01 `since` cursor is second-granularity, so the request is
 * intentionally inclusive at `frontier.createdAt`; the client then drops known
 * duplicates and keeps unknown frontier-second events too. That closes the
 * history→live race even when a new same-second event sorts before the current
 * frontier id, while normal id-dedupe keeps duplicate delivery harmless.
 */
export async function frontierBridge({
  frontier,
  isActive = () => true,
  knownEventIds,
  limit,
  onEvent,
  requestHistory,
  targetFilter,
}: {
  frontier: EventFrontier | null;
  isActive?: () => boolean;
  limit?: number;
  knownEventIds?: ReadonlySet<string>;
  onEvent: (event: RelayEvent) => void;
  requestHistory: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  targetFilter: RelaySubscriptionFilter;
}) {
  if (!frontier) {
    return;
  }

  const knownIds = knownEventIds ?? new Set<string>();

  const events = await requestHistory(
    buildFrontierBridgeFilter({ frontier, limit, targetFilter }),
  );

  if (!isActive()) {
    return;
  }

  for (const event of sortEvents(events).filter((event) =>
    shouldApplyBridgeEvent({ event, frontier, knownEventIds: knownIds }),
  )) {
    if (!isActive()) {
      return;
    }
    onEvent(event);
  }
}
