import type { RelayEvent, SearchHit } from "@/shared/api/types";

const MAX_CACHED_EVENTS = 200;
const searchHitEventCache = new Map<string, RelayEvent>();

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

export function cacheSearchHitEvent(hit: SearchHit): RelayEvent {
  const event = buildSearchHitEvent(hit);
  searchHitEventCache.set(event.id, event);
  trimCache();
  return event;
}

export function clearSearchHitEventCache(): void {
  searchHitEventCache.clear();
}

export function getCachedSearchHitEvent(
  eventId: string | null | undefined,
): RelayEvent | null {
  if (!eventId) {
    return null;
  }

  return searchHitEventCache.get(eventId) ?? null;
}
