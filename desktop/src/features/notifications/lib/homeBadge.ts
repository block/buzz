import type { FeedItem } from "@/shared/api/types";

export function shouldCountTowardHomeBadgeSubtotal(
  item: Pick<FeedItem, "channelId">,
  highPriorityChannelIds: ReadonlySet<string>,
): boolean {
  return item.channelId === null || !highPriorityChannelIds.has(item.channelId);
}
