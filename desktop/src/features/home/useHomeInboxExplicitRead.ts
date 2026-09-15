import * as React from "react";

import type { InboxItem } from "@/features/home/lib/inbox";
import { findInboxItemByEventId } from "@/features/home/lib/inbox";
import { resolveInboxCompletionSelection } from "@/features/home/lib/inboxSelection";
import type { HistorySearchSetterOptions } from "@/shared/hooks/useHistorySearchState";

type UseHomeInboxExplicitReadOptions = {
  applyInboxSearchPatch: (
    patch: { item: null },
    options: HistorySearchSetterOptions,
  ) => void;
  filteredItems: readonly Pick<InboxItem, "conversationId" | "id">[];
  inboxItems: readonly InboxItem[];
  isNarrowHomeViewport: boolean;
  markItemRead: (itemId: string) => void;
  selectedConversationId: string | null;
  setAutoSelectedEventId: React.Dispatch<React.SetStateAction<string | null>>;
  setUnreadBoundary: React.Dispatch<
    React.SetStateAction<{ conversationId: string; eventId: string } | null>
  >;
  unreadOnly: boolean;
};

/**
 * Marks an Inbox item read in response to an explicit user action.
 *
 * Under the unread-only filter, reading the active conversation removes it from
 * the list, so the selection is advanced to the next valid row (or handed back
 * to the list on narrow layouts) in the same update. That keeps the detail pane
 * from rendering a conversation that is no longer visible.
 */
export function useHomeInboxExplicitRead({
  applyInboxSearchPatch,
  filteredItems,
  inboxItems,
  isNarrowHomeViewport,
  markItemRead,
  selectedConversationId,
  setAutoSelectedEventId,
  setUnreadBoundary,
  unreadOnly,
}: UseHomeInboxExplicitReadOptions) {
  return React.useCallback(
    (itemId: string) => {
      const completedItem = findInboxItemByEventId(inboxItems, itemId);
      if (
        unreadOnly &&
        completedItem?.conversationId === selectedConversationId
      ) {
        const nextSelectedEventId = resolveInboxCompletionSelection({
          completedConversationId: completedItem.conversationId,
          isNarrow: isNarrowHomeViewport,
          items: filteredItems,
        });
        setUnreadBoundary(null);
        setAutoSelectedEventId(nextSelectedEventId);
        applyInboxSearchPatch({ item: null }, { replace: true });
      }
      markItemRead(itemId);
    },
    [
      applyInboxSearchPatch,
      filteredItems,
      inboxItems,
      isNarrowHomeViewport,
      markItemRead,
      selectedConversationId,
      setAutoSelectedEventId,
      setUnreadBoundary,
      unreadOnly,
    ],
  );
}
