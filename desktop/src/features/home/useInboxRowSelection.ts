import * as React from "react";

import type { InboxItem } from "@/features/home/lib/inbox";
import type { InboxOpenScrollIntent } from "@/features/home/lib/inboxOpenScroll";
import {
  hasGroupedUnreadOverride,
  resolveInboxItemReadAt,
} from "@/features/home/useHomeInboxReadState";
import {
  getThreadReference,
  isThreadReply,
} from "@/features/messages/lib/threading";

type UseInboxRowSelectionOptions = {
  doneSet: ReadonlySet<string>;
  getChannelReadAt: (channelId: string) => number | null;
  getMessageReadAt: (messageId: string) => number | null;
  getThreadReadAt: (rootId: string, channelId?: string | null) => number | null;
  items: readonly InboxItem[];
  localUnreadSet: ReadonlySet<string>;
  markItemRead: (itemId: string) => void;
  selectItem: (itemId: string | null) => void;
};

export function useInboxRowSelection({
  doneSet,
  getChannelReadAt,
  getMessageReadAt,
  getThreadReadAt,
  items,
  localUnreadSet,
  markItemRead,
  selectItem,
}: UseInboxRowSelectionOptions) {
  const requestIdRef = React.useRef(0);
  const [openScrollIntent, setOpenScrollIntent] =
    React.useState<InboxOpenScrollIntent | null>(null);
  const itemById = React.useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  );

  const selectInboxRow = React.useCallback(
    (itemId: string) => {
      const item = itemById.get(itemId);
      if (item) {
        requestIdRef.current += 1;
        const wasUnread = !doneSet.has(item.id);
        const threadRootId = isThreadReply(item.item.tags)
          ? getThreadReference(item.item.tags).rootId
          : null;
        setOpenScrollIntent({
          anchorEventId: item.id,
          conversationId: item.conversationId,
          excludedRootId: threadRootId,
          forcedUnreadMessageId:
            wasUnread && hasGroupedUnreadOverride(item, localUnreadSet)
              ? item.id
              : null,
          openReadAt: wasUnread
            ? resolveInboxItemReadAt(item, {
                getChannelReadAt,
                getMessageReadAt,
                getThreadReadAt,
              })
            : null,
          requestId: requestIdRef.current,
          wasUnread,
        });
      }
      selectItem(itemId);
      markItemRead(itemId);
    },
    [
      doneSet,
      getChannelReadAt,
      getMessageReadAt,
      getThreadReadAt,
      itemById,
      localUnreadSet,
      markItemRead,
      selectItem,
    ],
  );

  return { openScrollIntent, selectInboxRow };
}
