import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { maxReadAt } from "@/features/channels/readState/readStateFormat";
import {
  getThreadReference,
  isBroadcastReply,
  isThreadReply,
} from "@/features/messages/lib/threading";
import { feedItemSurvivesChannelMute } from "@/features/notifications/lib/feed";

function dedupeFeedItemsById(items: readonly FeedItem[]): FeedItem[] {
  const seen = new Set<string>();
  const result: FeedItem[] = [];
  for (const item of items) {
    if (seen.has(item.id)) {
      continue;
    }
    seen.add(item.id);
    result.push(item);
  }
  return result;
}

export function buildHomeBadgeFeedItems(
  feed: HomeFeedResponse | undefined,
  extraInboxItems: readonly FeedItem[],
  localUnreadFeedIds: ReadonlySet<string>,
): FeedItem[] {
  const items = feed
    ? [...feed.feed.mentions, ...feed.feed.needsAction, ...extraInboxItems]
    : [...extraInboxItems];

  if (feed && localUnreadFeedIds.size > 0) {
    items.push(
      ...feed.feed.activity.filter((item) => localUnreadFeedIds.has(item.id)),
      ...feed.feed.agentActivity.filter((item) =>
        localUnreadFeedIds.has(item.id),
      ),
    );
  }

  return dedupeFeedItemsById(items);
}

export type HomeBadgeCounts = {
  homeBadgeCount: number;
  homeBadgeCountExcludingHighPriority: number;
};

/**
 * Count the unread rows the home badge should show.
 *
 * Pure counterpart of the badge memo in `useHomeFeedNotificationState`. Mute
 * suppression runs through the shared `feedItemSurvivesChannelMute` predicate,
 * so the badge and the toast path apply the same NIP-CM ladder: a direct
 * mention pierces a channel mute, a channel-wide `@channel`/`@here` marker
 * does not.
 */
export function countHomeBadgeFeedItems(
  items: readonly FeedItem[],
  options: {
    currentPubkey?: string;
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
    highPriorityChannelIds: ReadonlySet<string>;
    isHomeActive: boolean;
    localUnreadFeedIds: ReadonlySet<string>;
    mutedChannelIds?: ReadonlySet<string>;
    seenFeedIdSet: ReadonlySet<string>;
  },
): HomeBadgeCounts {
  let homeBadgeCount = 0;
  let homeBadgeCountExcludingHighPriority = 0;

  for (const item of items) {
    const isLocallyUnread = options.localUnreadFeedIds.has(item.id);
    if (options.isHomeActive && !isLocallyUnread) {
      continue;
    }
    if (
      !feedItemSurvivesChannelMute(
        item,
        options.mutedChannelIds,
        options.currentPubkey,
      )
    ) {
      continue;
    }
    const isUnread = isHomeBadgeFeedItemUnread(item, {
      getChannelReadAt: options.getChannelReadAt,
      getMessageReadAt: options.getMessageReadAt,
      getThreadReadAt: options.getThreadReadAt,
      isLocallyUnread,
      seenFeedIdSet: options.seenFeedIdSet,
    });
    if (!isUnread) {
      continue;
    }
    homeBadgeCount++;
    if (
      shouldCountTowardHomeBadgeSubtotal(
        item,
        options.highPriorityChannelIds,
        isLocallyUnread,
      )
    ) {
      homeBadgeCountExcludingHighPriority++;
    }
  }

  return { homeBadgeCount, homeBadgeCountExcludingHighPriority };
}

export function shouldCountTowardHomeBadgeSubtotal(
  item: Pick<FeedItem, "channelId" | "channelType" | "tags">,
  highPriorityChannelIds: ReadonlySet<string>,
  forceHomeCount = false,
): boolean {
  if (forceHomeCount) {
    return true;
  }

  if (item.channelId === null || !highPriorityChannelIds.has(item.channelId)) {
    return true;
  }

  const threadRef = getThreadReference(item.tags);
  const isThreadedReply =
    threadRef.parentId !== null && !isBroadcastReply(item.tags);
  return isThreadedReply && item.channelType !== "dm";
}

type FeedItemReadState = Pick<
  FeedItem,
  "channelId" | "createdAt" | "id" | "tags"
>;

export function feedItemThreadRootId(item: Pick<FeedItem, "tags">) {
  return isThreadReply(item.tags) ? getThreadReference(item.tags).rootId : null;
}

export function isHomeBadgeFeedItemUnread(
  item: FeedItemReadState,
  options: {
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
    isLocallyUnread?: boolean;
    seenFeedIdSet: ReadonlySet<string>;
  },
): boolean {
  if (options.isLocallyUnread) {
    return true;
  }

  const readAt = resolveHomeBadgeFeedItemReadAt(item, options);
  return readAt !== null
    ? item.createdAt > readAt
    : !options.seenFeedIdSet.has(item.id);
}

export function resolveHomeBadgeFeedItemReadAt(
  item: FeedItemReadState,
  options: {
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
  },
): number | null {
  const threadRootId = feedItemThreadRootId(item);
  const markers: Array<number | null> = [];

  if (item.channelId && !threadRootId) {
    markers.push(options.getChannelReadAt(item.channelId));
  }
  if (threadRootId) {
    markers.push(options.getThreadReadAt(threadRootId, item.channelId));
    markers.push(options.getMessageReadAt?.(item.id) ?? null);
  }

  return maxReadAt(...markers);
}
