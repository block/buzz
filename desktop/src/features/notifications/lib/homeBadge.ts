import type { FeedItem } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";

export function shouldCountTowardHomeBadgeSubtotal(
  item: Pick<FeedItem, "channelId" | "channelType" | "tags">,
  highPriorityChannelIds: ReadonlySet<string>,
): boolean {
  if (item.channelId === null || !highPriorityChannelIds.has(item.channelId)) {
    return true;
  }

  const threadRef = getThreadReference(item.tags);
  const isThreadedReply =
    threadRef.parentId !== null && !isBroadcastReply(item.tags);
  return isThreadedReply && item.channelType !== "dm";
}
