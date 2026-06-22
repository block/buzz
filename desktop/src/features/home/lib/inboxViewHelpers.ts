import type { InboxFilter } from "@/features/home/lib/inbox";
import {
  getChannelIdFromTags,
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import type { FeedItem, RelayEvent } from "@/shared/api/types";

function hasThreadReplyTags(tags: string[][]) {
  const thread = getThreadReference(tags);
  return thread.parentId !== null && !isBroadcastReply(tags);
}

export function matchesInboxFilter(
  item: {
    categories: readonly string[];
    groupItems?: readonly FeedItem[];
    item?: FeedItem;
  },
  filter: InboxFilter,
) {
  if (filter === "all") {
    return true;
  }

  if (filter === "thread") {
    return [item.item, ...(item.groupItems ?? [])].some((groupItem) =>
      groupItem ? hasThreadReplyTags(groupItem.tags) : false,
    );
  }

  return item.categories.includes(filter);
}

export function getContextMessageDepth(
  event: RelayEvent,
  eventById: ReadonlyMap<string, RelayEvent>,
): number {
  let depth = 0;
  let parentId = getThreadReference(event.tags).parentId;
  const seen = new Set<string>([event.id]);

  while (parentId && eventById.has(parentId) && !seen.has(parentId)) {
    depth += 1;
    seen.add(parentId);
    parentId = getThreadReference(eventById.get(parentId)?.tags ?? []).parentId;
  }

  return depth;
}

export function isInboxThreadContextEvent(
  event: RelayEvent,
  selection: {
    selectedChannelId: string | null;
    selectedEventId: string;
    selectedParentId: string | null;
    selectedThreadRootId: string | null;
  },
): boolean {
  if (
    selection.selectedChannelId &&
    getChannelIdFromTags(event.tags) !== selection.selectedChannelId
  ) {
    return false;
  }

  if (event.id === selection.selectedEventId) {
    return true;
  }

  if (
    selection.selectedThreadRootId &&
    event.id === selection.selectedThreadRootId
  ) {
    return true;
  }

  if (selection.selectedParentId && event.id === selection.selectedParentId) {
    return true;
  }

  const thread = getThreadReference(event.tags);
  return (
    (selection.selectedThreadRootId !== null &&
      (thread.rootId === selection.selectedThreadRootId ||
        thread.parentId === selection.selectedThreadRootId)) ||
    thread.parentId === selection.selectedEventId
  );
}

export function getReactionTargetId(tags: string[][]) {
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const tag = tags[index];
    if (tag?.[0] === "e" && typeof tag[1] === "string") {
      return tag[1];
    }
  }

  return null;
}
