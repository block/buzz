export type ObservedUnreadEvent = {
  id: string;
  createdAt: number;
};

export function mapsEqual(
  a: ReadonlyMap<string, number>,
  b: ReadonlyMap<string, number>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [key, value] of a) {
    if (b.get(key) !== value) return false;
  }
  return true;
}

export function recordObservedUnreadEvent(
  eventsByChannel: Map<string, Map<string, number>>,
  channelId: string,
  event: ObservedUnreadEvent,
  limit: number,
): void {
  let eventsById = eventsByChannel.get(channelId);
  if (!eventsById) {
    eventsById = new Map<string, number>();
    eventsByChannel.set(channelId, eventsById);
  }
  if (eventsById.has(event.id)) return;

  eventsById.set(event.id, event.createdAt);
  if (eventsById.size <= limit) return;

  const oldest = [...eventsById.entries()].sort((a, b) => a[1] - b[1])[0]?.[0];
  if (oldest) {
    eventsById.delete(oldest);
  }
}

export function countUnreadObservedEvents(
  eventsById: ReadonlyMap<string, number> | undefined,
  readAt: number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const createdAt of eventsById.values()) {
    if (readAt === null || createdAt > readAt) count += 1;
  }
  return count;
}

export function buildChannelThreadRoots<
  T extends { channelId: string; tags: string[][] },
>(
  items: readonly T[],
  getRootId: (tags: string[][]) => string | null,
): Map<string, Set<string>> {
  const byChannel = new Map<string, Set<string>>();
  for (const item of items) {
    const rootId = getRootId(item.tags);
    if (rootId === null) continue;
    let roots = byChannel.get(item.channelId);
    if (!roots) {
      roots = new Set<string>();
      byChannel.set(item.channelId, roots);
    }
    roots.add(rootId);
  }
  return byChannel;
}

export function channelUnreadFrontier(
  channelMarker: number | null,
  threadRoots: ReadonlySet<string> | undefined,
  getThreadOwnMarker: (rootId: string) => number | null,
): number | null {
  let frontier = channelMarker;
  if (threadRoots) {
    for (const rootId of threadRoots) {
      const own = getThreadOwnMarker(rootId);
      if (own !== null && (frontier === null || own > frontier)) {
        frontier = own;
      }
    }
  }
  return frontier;
}
