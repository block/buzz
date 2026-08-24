import type { RelayEvent, SearchHit } from "@/shared/api/types";

const MAX_CACHED_EVENTS = 200;
const searchHitEventCache = new Map<string, RelayEvent>();
const searchHitQueryCache = new Map<string, string>();

function trimCache() {
  if (searchHitEventCache.size <= MAX_CACHED_EVENTS) {
    return;
  }

  const overflow = searchHitEventCache.size - MAX_CACHED_EVENTS;
  let removed = 0;
  for (const key of searchHitEventCache.keys()) {
    if (removed >= overflow) {
      break;
    }
    searchHitEventCache.delete(key);
    searchHitQueryCache.delete(key);
    removed++;
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
): RelayEvent {
  const event = buildSearchHitEvent(hit);
  searchHitEventCache.set(event.id, event);
  const trimmedQuery = query?.trim();
  if (trimmedQuery) {
    searchHitQueryCache.set(event.id, trimmedQuery);
  } else {
    searchHitQueryCache.delete(event.id);
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
  eventId: string | null | undefined,
): string | null {
  if (!eventId) {
    return null;
  }

  const query = searchHitQueryCache.get(eventId) ?? null;
  searchHitQueryCache.delete(eventId);
  return query;
}
