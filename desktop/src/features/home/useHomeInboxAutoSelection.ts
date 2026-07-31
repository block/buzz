import * as React from "react";

import type { InboxItem } from "@/features/home/lib/inbox";

type UseHomeInboxAutoSelectionOptions = {
  coldResolutionPending: boolean;
  filteredItems: readonly Pick<InboxItem, "conversationId" | "id">[];
  hasFeed: boolean;
  hasPersonalSelection: boolean;
  homeInboxWidthPx: number;
  isLoading: boolean;
  isMessagesMode: boolean;
  isNarrowHomeViewport: boolean;
  selectedConversationId: string | null;
  setAutoSelectedEventId: React.Dispatch<React.SetStateAction<string | null>>;
  urlSelectedItemId: string | null;
};

export function useHomeInboxAutoSelection({
  coldResolutionPending,
  filteredItems,
  hasFeed,
  hasPersonalSelection,
  homeInboxWidthPx,
  isLoading,
  isMessagesMode,
  isNarrowHomeViewport,
  selectedConversationId,
  setAutoSelectedEventId,
  urlSelectedItemId,
}: UseHomeInboxAutoSelectionOptions) {
  React.useEffect(() => {
    if (!isMessagesMode) return;

    if (hasPersonalSelection || urlSelectedItemId !== null) {
      setAutoSelectedEventId(null);
      return;
    }

    if (isLoading || !hasFeed) return;

    if (filteredItems.length === 0) {
      setAutoSelectedEventId(null);
      return;
    }

    // Wait for the width measurement so narrow Home does not cold-load detail.
    if (homeInboxWidthPx === 0) return;

    const selectedConversationIsVisible =
      selectedConversationId !== null &&
      filteredItems.some(
        (item) => item.conversationId === selectedConversationId,
      );
    if (selectedConversationIsVisible || coldResolutionPending) return;

    setAutoSelectedEventId(
      isNarrowHomeViewport ? null : (filteredItems[0]?.id ?? null),
    );
  }, [
    coldResolutionPending,
    filteredItems,
    hasFeed,
    hasPersonalSelection,
    homeInboxWidthPx,
    isLoading,
    isMessagesMode,
    isNarrowHomeViewport,
    selectedConversationId,
    setAutoSelectedEventId,
    urlSelectedItemId,
  ]);
}
