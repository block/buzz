import type { RelayEvent, SearchHit } from "@/shared/api/types";

const MAX_CACHED_EVENTS = 200;
const searchHitEventCache = new Map<string, RelayEvent>();
const searchHitQueryCache = new Map<
  string,
  { eventId: string; query: string }
>();

function trimCache() {
  const eventOverflow = searchHitEventCache.size - MAX_CACHED_EVENTS;
  let removedEvents = 0;
  for (const key of searchHitEventCache.keys()) {
    if (removedEvents >= eventOverflow) {
      break;
    }
    searchHitEventCache.delete(key);
    for (const [navigationId, entry] of searchHitQueryCache) {
      if (entry.eventId === key) {
        searchHitQueryCache.delete(navigationId);
      }
    }
    removedEvents++;
  }

  const queryOverflow = searchHitQueryCache.size - MAX_CACHED_EVENTS;
  let removedQueries = 0;
  for (const navigationId of searchHitQueryCache.keys()) {
    if (removedQueries >= queryOverflow) {
      break;
    }
    searchHitQueryCache.delete(navigationId);
    removedQueries++;
  }
}

export function buildSearchHitEvent(hit: SearchHit): RelayEvent {
  return {
    id: hit.eventId,
    pubkey: hit.pubkey,
    created_at: hit.createdAt,
    kind: hit.kind,
    tags: hit.channelId ? [["h", hit.channelId]] : [],
    content: hit.content,
    sig: "",
  };
}

export function cacheSearchHitEvent(
  hit: SearchHit,
  query?: string,
  searchNavigationId = hit.eventId,
): RelayEvent {
  const event = buildSearchHitEvent(hit);
  searchHitEventCache.set(event.id, event);
  const trimmedQuery = query?.trim();
  if (trimmedQuery) {
    searchHitQueryCache.set(searchNavigationId, {
      eventId: event.id,
      query: trimmedQuery,
    });
  } else {
    searchHitQueryCache.delete(searchNavigationId);
  }
  trimCache();
  return event;
}

export function clearSearchHitEventCache(): void {
  searchHitEventCache.clear();
  searchHitQueryCache.clear();
}

export function getCachedSearchHitEvent(
  eventId: string | null | undefined,
): RelayEvent | null {
  if (!eventId) {
    return null;
  }

  return searchHitEventCache.get(eventId) ?? null;
}

export function consumeCachedSearchHitQuery(
  searchNavigationId: string | null | undefined,
): { eventId: string; query: string } | null {
  if (!searchNavigationId) {
    return null;
  }

  const entry = searchHitQueryCache.get(searchNavigationId) ?? null;
  searchHitQueryCache.delete(searchNavigationId);
  return entry;
}
