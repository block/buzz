import { maxReadAt } from "@/features/channels/readState/readStateFormat";

export type ObservedUnreadEvent = {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  isDm: boolean;
  isThreadedReply: boolean;
  countsTowardBadge: boolean;
  countsTowardAppBadge: boolean;
};

export function makeObservedUnreadEvent(input: {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  channelType: string | undefined;
  isThreadedReply: boolean;
}): ObservedUnreadEvent {
  const isDm = input.channelType === "dm";
  return {
    id: input.id,
    createdAt: input.createdAt,
    rootId: input.rootId,
    highPriority: input.highPriority,
    isDm,
    isThreadedReply: input.isThreadedReply,
    countsTowardBadge: isDm || input.isThreadedReply || input.highPriority,
    countsTowardAppBadge:
      isDm || (!input.isThreadedReply && input.highPriority),
  };
}

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

export function countUnreadBadgeObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.countsTowardBadge) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export function countUnreadAppBadgeObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.countsTowardAppBadge) continue;
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

/**
 * A badge event whose only reason for counting is a threaded reply: not a DM
 * and not a mention/broadcast. A mention inside a thread is high-priority and
 * is intentionally excluded — it keeps the solid numeric badge.
 */
export function isThreadOnlyBadgeObservedEvent(
  event: ObservedUnreadEvent,
): boolean {
  return event.isThreadedReply === true && !event.isDm && !event.highPriority;
}

/**
 * True when at least one badge-counting event is unread and ALL unread
 * badge-counting events are thread-only — i.e. the channel's numeric badge is
 * entirely explained by replies invisible in the channel scroll, so the
 * sidebar can demote it to the thread-glyph badge.
 */
export function unreadBadgeObservedEventsAreThreadOnly(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): boolean {
  if (!eventsById) return false;
  let sawThreadOnly = false;
  for (const event of eventsById.values()) {
    if (!event.countsTowardBadge) continue;
    const readAt = getReadAt(event);
    if (readAt !== null && event.createdAt <= readAt) continue;
    if (!isThreadOnlyBadgeObservedEvent(event)) return false;
    sawThreadOnly = true;
  }
  return sawThreadOnly;
}

export function observedUnreadEventReadAt(
  event: ObservedUnreadEvent,
  channelReadAt: number | null,
  getThreadOwnMarker: (rootId: string) => number | null,
  getMessageOwnMarker: (messageId: string) => number | null = () => null,
): number | null {
  const markers = [channelReadAt, getMessageOwnMarker(event.id)];

  if (event.rootId !== null) {
    markers.push(getThreadOwnMarker(event.rootId));
  }

  return maxReadAt(...markers);
}
