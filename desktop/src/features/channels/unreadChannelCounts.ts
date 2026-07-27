import { maxReadAt } from "@/features/channels/readState/readStateFormat";

export type ObservedUnreadEvent = {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  /**
   * True for a direct `p`-tag mention of the current user. Unlike
   * `highPriority`, this is state-independent, so it stays correct after the
   * channel's NIP-CN level changes — which is what makes it safe to freeze on
   * the record. A muted channel's mention tier is keyed off this, not
   * `highPriority`: an `@channel` marker or a NIP-CW broadcast reply earns
   * `highPriority` only from the level it was observed under, and muting the
   * channel afterwards must retire it.
   */
  directMention: boolean;
  countsTowardBadge: boolean;
  countsTowardAppBadge: boolean;
};

export function makeObservedUnreadEvent(input: {
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  directMention: boolean;
  channelType: string | undefined;
  isThreadedReply: boolean;
}): ObservedUnreadEvent {
  const isDm = input.channelType === "dm";
  return {
    id: input.id,
    createdAt: input.createdAt,
    rootId: input.rootId,
    highPriority: input.highPriority,
    directMention: input.directMention,
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

export function countUnreadDirectMentionObservedEvents(
  eventsById: ReadonlyMap<string, ObservedUnreadEvent> | undefined,
  getReadAt: (event: ObservedUnreadEvent) => number | null,
): number {
  if (!eventsById) return 0;
  let count = 0;
  for (const event of eventsById.values()) {
    if (!event.directMention) continue;
    const readAt = getReadAt(event);
    if (readAt === null || event.createdAt > readAt) count += 1;
  }
  return count;
}

export type UnreadAggregation = {
  unreadChannelIds: Set<string>;
  highPriorityUnreadChannelIds: Set<string>;
  unreadChannelCounts: Map<string, number>;
  unreadChannelNotificationCount: number;
};

/**
 * Project the observed-unread evidence onto the sidebar's per-channel tiers.
 *
 * Pure: the caller supplies the read markers, the observed events, and the
 * NIP-CN mute predicate, which is evaluated against the channel's *current*
 * resolved level — so changing a level re-tiers immediately without rewriting
 * the frozen `ObservedUnreadEvent` records (and therefore never re-notifies).
 * A muted channel contributes only its **direct-mention** events, mirroring the
 * sidebar escape hatch that keeps a hidden channel visible while it holds a
 * mention. It deliberately does not use the frozen `highPriority` flag: that was
 * decided under the level in force when the event arrived, so a broadcast reply
 * or `@channel` marker observed at level "all" would otherwise keep badging (and
 * un-hiding) the channel after the user mutes it. Direct mentions pierce every
 * level, so their frozen flag stays correct forever.
 */
export function aggregateUnreadChannels(input: {
  channels: readonly { id: string; channelType?: string }[];
  activeChannelId: string | null;
  hasForcedUnread: (channelId: string) => boolean;
  hasObservedLatest: (channelId: string) => boolean;
  getObservedEvents: (
    channelId: string,
  ) => ReadonlyMap<string, ObservedUnreadEvent> | undefined;
  getReadAt: (
    channelId: string,
  ) => (event: ObservedUnreadEvent) => number | null;
  isMutedChannel: (channelId: string) => boolean;
}): UnreadAggregation {
  const unread = new Set<string>();
  const highPriority = new Set<string>();
  const counts = new Map<string, number>();
  let unreadChannelNotificationCount = 0;

  for (const channel of input.channels) {
    if (channel.id === input.activeChannelId) continue;

    // DMs bypass NIP-CN levels entirely.
    const isMuted =
      channel.channelType !== "dm" && input.isMutedChannel(channel.id);

    if (input.hasForcedUnread(channel.id)) {
      if (isMuted) continue;
      // Forced-unread is dot tier only — not high-priority.
      unread.add(channel.id);
      counts.set(channel.id, 1);
      unreadChannelNotificationCount += 1;
      continue;
    }

    if (!input.hasObservedLatest(channel.id)) continue;

    const observedEvents = input.getObservedEvents(channel.id);
    const readAtFor = input.getReadAt(channel.id);

    if (countUnreadObservedEvents(observedEvents, readAtFor) === 0) continue;

    // Muted channels are tiered on direct mentions only (see the doc comment);
    // unmuted channels use the frozen decision from the ladder.
    const mentionTierCount = isMuted
      ? countUnreadDirectMentionObservedEvents(observedEvents, readAtFor)
      : countUnreadHighPriorityObservedEvents(observedEvents, readAtFor);
    if (isMuted && mentionTierCount === 0) continue;

    unread.add(channel.id);
    counts.set(
      channel.id,
      isMuted
        ? mentionTierCount
        : countUnreadBadgeObservedEvents(observedEvents, readAtFor),
    );
    unreadChannelNotificationCount += isMuted
      ? mentionTierCount
      : countUnreadAppBadgeObservedEvents(observedEvents, readAtFor);

    // DM channels: any unread DM is high-priority. Non-DM: only when at least
    // one mention/broadcast remains unread in its own channel/thread context.
    if (channel.channelType === "dm" || mentionTierCount > 0) {
      highPriority.add(channel.id);
    }
  }

  return {
    unreadChannelIds: unread,
    highPriorityUnreadChannelIds: highPriority,
    unreadChannelCounts: counts,
    unreadChannelNotificationCount,
  };
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
