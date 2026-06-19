export type ObservedUnreadEvent = {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
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
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
  channelId: string,
  event: ObservedUnreadEvent,
  limit: number,
): boolean {
  let eventsById = eventsByChannel.get(channelId);
  if (!eventsById) {
    eventsById = new Map<string, ObservedUnreadEvent>();
    eventsByChannel.set(channelId, eventsById);
  }
  if (eventsById.has(event.id)) return false;

  eventsById.set(event.id, event);
  if (eventsById.size <= limit) return true;

  const oldest = [...eventsById.values()].sort(
    (a, b) => a.createdAt - b.createdAt,
  )[0]?.id;
  if (oldest) {
    eventsById.delete(oldest);
  }
  return true;
}

export function countUnreadObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function countUnreadHighPriorityObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.highPriority) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function observedUnreadEventReadAt(
  event: ObservedUnreadEvent,
  channelReadAt: number | null,
  getThreadOwnMarker: (rootId: string) => number | null,
): number | null {
  if (event.rootId === null) return channelReadAt;

  const threadReadAt = getThreadOwnMarker(event.rootId);
  if (threadReadAt === null) return channelReadAt;
  if (channelReadAt === null || threadReadAt > channelReadAt) {
    return threadReadAt;
  }
  return channelReadAt;
}
