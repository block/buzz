import type { ObservedUnreadEvent } from "./unreadChannelCounts";
import { maxReadAt } from "./readState/readStateFormat";
import type { ObservedUnreadProjection } from "@/shared/api/tauriObservedUnread";

export function collectMarkAllUnreadChannelIds(
  visibleUnreadChannelIds: ReadonlySet<string>,
  nativeProjections: ReadonlyMap<string, ObservedUnreadProjection>,
): Set<string> {
  const result = new Set(visibleUnreadChannelIds);
  for (const [channelId, projection] of nativeProjections) {
    if (projection.unreadThreadEventIds.length > 0) result.add(channelId);
  }
  return result;
}

export function collectUnreadThreadEventIds(
  nativeEventIds: readonly string[] | undefined,
  observedEvents: Iterable<ObservedUnreadEvent> | undefined,
  readAtForObservedEvent: (event: ObservedUnreadEvent) => number | null,
): Set<string> {
  if (nativeEventIds) return new Set(nativeEventIds);
  const ids = new Set<string>();
  for (const event of observedEvents ?? []) {
    if (
      event.rootId !== null &&
      event.createdAt > (readAtForObservedEvent(event) ?? 0)
    ) {
      ids.add(event.id);
    }
  }
  return ids;
}

export function readAtForPreservedUnreadMessage(
  messageId: string,
  preservedUnreadMessageIds: ReadonlySet<string>,
  preservedUnreadMessageIdsOnFirstOpen: ReadonlySet<string>,
  createdAt: number | undefined,
  openFrontierSeconds: number | null,
  messageReadAt: number | null,
  channelReadAt: number | null,
): number | null {
  if (
    preservedUnreadMessageIds.has(messageId) &&
    (preservedUnreadMessageIdsOnFirstOpen.has(messageId) ||
      (createdAt !== undefined &&
        (openFrontierSeconds === null || createdAt > openFrontierSeconds)))
  ) {
    return messageReadAt;
  }
  return maxReadAt(messageReadAt, channelReadAt);
}
