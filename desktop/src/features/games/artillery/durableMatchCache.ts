import type { RelayEvent } from "@/shared/api/types";

const CACHE_PREFIX = "buzz-artillery-durable.v1:";

function cacheKey(channelId: string, rootEventId: string) {
  return `${CACHE_PREFIX}${channelId}:${rootEventId}`;
}

/** Reads the last locally validated relay records for fast/offline hydration. */
export function readDurableMatchCache(
  channelId: string,
  rootEventId: string,
): RelayEvent[] {
  try {
    const value: unknown = JSON.parse(
      window.localStorage.getItem(cacheKey(channelId, rootEventId)) ?? "[]",
    );
    if (!Array.isArray(value)) return [];
    return value.filter(
      (event): event is RelayEvent =>
        Boolean(event) &&
        typeof event === "object" &&
        typeof (event as RelayEvent).id === "string" &&
        typeof (event as RelayEvent).content === "string" &&
        typeof (event as RelayEvent).created_at === "number",
    );
  } catch {
    return [];
  }
}

/** Caches a canonical match event after it has passed protocol validation. */
export function cacheDurableMatchEvent(
  channelId: string,
  rootEventId: string,
  event: RelayEvent,
) {
  const events = readDurableMatchCache(channelId, rootEventId);
  const byId = new Map(events.map((entry) => [entry.id, entry]));
  byId.set(event.id, event);
  window.localStorage.setItem(
    cacheKey(channelId, rootEventId),
    JSON.stringify([...byId.values()]),
  );
}

/** Clears cached match events at a community boundary. */
export function resetDurableMatchCache() {
  for (let index = window.localStorage.length - 1; index >= 0; index -= 1) {
    const key = window.localStorage.key(index);
    if (key?.startsWith(CACHE_PREFIX)) window.localStorage.removeItem(key);
  }
}
