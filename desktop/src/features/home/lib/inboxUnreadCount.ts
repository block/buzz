import type { InboxItem } from "@/features/home/lib/inbox";
import {
  hasGroupedUnreadOverride,
  getInboxThreadRootId,
  resolveInboxItemReadAt,
} from "@/features/home/useHomeInboxReadState";
import { matchesInboxFilter } from "@/features/home/lib/inboxViewHelpers";

/**
 * Same projection as `useHomeInboxReadState`'s `effectiveDoneSet`: NIP-RS
 * markers win; local done-set is a fallback for non-channel items; local
 * unread overrides keep a row out of the done set.
 */
export function projectInboxEffectiveDoneSet(
  items: readonly InboxItem[],
  options: {
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
    localDoneSet: ReadonlySet<string>;
    localUnreadSet?: ReadonlySet<string>;
  },
): ReadonlySet<string> {
  const localUnreadSet = options.localUnreadSet ?? new Set<string>();
  const result = new Set<string>();
  for (const item of items) {
    if (hasGroupedUnreadOverride(item, localUnreadSet)) {
      continue;
    }

    const threadRootId = getInboxThreadRootId(item);
    const readAt = resolveInboxItemReadAt(item, {
      getChannelReadAt: options.getChannelReadAt,
      getMessageReadAt: options.getMessageReadAt,
      getThreadReadAt: options.getThreadReadAt,
    });
    if (readAt !== null) {
      if (item.latestActivityAt <= readAt) {
        result.add(item.id);
      }
      continue;
    }

    if (threadRootId !== null || item.item.channelId) {
      continue;
    }

    if (options.localDoneSet.has(item.id)) {
      result.add(item.id);
    }
  }
  return result;
}

/** Count of inbox rows that are not in the effective done set (Mark-all-read numeral). */
export function countUnreadInboxItems(
  items: readonly InboxItem[],
  doneSet: ReadonlySet<string>,
): number {
  let count = 0;
  for (const item of items) {
    if (!doneSet.has(item.id)) count += 1;
  }
  return count;
}

/**
 * Sidebar Inbox badge source: Inbox "all" filter rows that are still unread
 * under the same done-set projection as InboxListPane (Unread default view).
 */
export function deriveSidebarInboxUnreadCount(input: {
  items: readonly InboxItem[];
  doneSet: ReadonlySet<string>;
  ownedAgentPubkeys: ReadonlySet<string>;
}): number {
  const visible = input.items.filter((item) =>
    matchesInboxFilter(item, "all", input.ownedAgentPubkeys),
  );
  return countUnreadInboxItems(visible, input.doneSet);
}
