import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { maxReadAt } from "@/features/channels/readState/readStateFormat";
import {
  getThreadReference,
  isBroadcastReply,
  isThreadReply,
} from "@/features/messages/lib/threading";

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
  // Thread activity is surfaced directly on its channel's hover preview. It
  // should not also inflate the Inbox numeral, which is reserved for the
  // Inbox's own high-priority activity.
  const nonThreadExtraInboxItems = extraInboxItems.filter(
    (item) => !isThreadReply(item.tags),
  );
  const items = feed
    ? [
        ...feed.feed.mentions,
        ...feed.feed.needsAction,
        ...nonThreadExtraInboxItems,
      ]
    : [...nonThreadExtraInboxItems];

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

export function homeMentionAttentionIds(
  feed: HomeFeedResponse | undefined,
): string[] {
  // The feed's mention bucket intentionally includes ordinary DMs because DM
  // messages address every other participant even without textual @mentions.
  return feed?.feed.mentions.map((item) => item.id) ?? [];
}

type HomeMentionReadState = {
  getChannelReadAt: (channelId: string) => number | null;
  getMessageReadAt?: (messageId: string) => number | null;
};

export function homeMentionSourceReadIds(
  feed: HomeFeedResponse | undefined,
  readState: HomeMentionReadState,
): string[] {
  return (
    feed?.feed.mentions
      .filter((item) => {
        const readAt = feedItemThreadRootId(item)
          ? (readState.getMessageReadAt?.(item.id) ?? null)
          : item.channelId
            ? readState.getChannelReadAt(item.channelId)
            : null;
        return readAt !== null && item.createdAt <= readAt;
      })
      .map((item) => item.id) ?? []
  );
}

export function hasUnseenHomeMention(
  feed: HomeFeedResponse | undefined,
  seenMentionIds: ReadonlySet<string>,
  readState?: HomeMentionReadState,
): boolean {
  const sourceReadIds = readState
    ? new Set(homeMentionSourceReadIds(feed, readState))
    : null;
  return homeMentionAttentionIds(feed).some(
    (id) => !seenMentionIds.has(id) && !sourceReadIds?.has(id),
  );
}

export function isInboxObserved(input: {
  appFocused: boolean;
  isHomeRoute: boolean;
  isHuddleRoom: boolean;
}): boolean {
  return input.isHomeRoute && input.appFocused && !input.isHuddleRoom;
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
