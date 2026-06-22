import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
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
