import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { PREVIEW_MAX_LENGTH } from "./bookmarksStorage";
import type { BookmarkTarget, UseBookmarks } from "./useBookmarks";

/**
 * The per-message toggle surface consumed by every message row. Split out from
 * the saved-list channel so that a no-op remote sync (which rebuilds the saved
 * array) never re-renders the timeline — only a genuine membership change to
 * `isBookmarked` does. `enabled` is false when no provider is mounted so
 * consumers can render standalone (tests, pre-auth).
 */
export type BookmarkActions = {
  enabled: boolean;
  isBookmarked: (eventId: string) => boolean;
  toggleBookmark: (target: BookmarkTarget) => void;
};

type SavedEntries = UseBookmarks["savedEntries"];

const DISABLED_ACTIONS: BookmarkActions = {
  enabled: false,
  isBookmarked: () => false,
  toggleBookmark: () => {},
};

const BookmarkActionsContext =
  React.createContext<BookmarkActions>(DISABLED_ACTIONS);
const SavedEntriesContext = React.createContext<SavedEntries>([]);

export function BookmarksProvider({
  value,
  children,
}: {
  value: UseBookmarks;
  children: React.ReactNode;
}) {
  const actions = React.useMemo<BookmarkActions>(
    () => ({
      enabled: true,
      isBookmarked: value.isBookmarked,
      toggleBookmark: value.toggleBookmark,
    }),
    [value.isBookmarked, value.toggleBookmark],
  );
  return (
    <BookmarkActionsContext.Provider value={actions}>
      <SavedEntriesContext.Provider value={value.savedEntries}>
        {children}
      </SavedEntriesContext.Provider>
    </BookmarkActionsContext.Provider>
  );
}

/** Timeline rows: the stable toggle + membership check. */
export function useBookmarkActions(): BookmarkActions {
  return React.useContext(BookmarkActionsContext);
}

/** The Saved view: the current user's bookmarked messages, newest-first. */
export function useSavedBookmarks(): SavedEntries {
  return React.useContext(SavedEntriesContext);
}

/** Build the persisted snapshot for a timeline message in a given channel. */
export function messageBookmarkTarget(
  message: TimelineMessage,
  channelId: string,
): BookmarkTarget {
  return {
    eventId: message.id,
    channelId,
    authorPubkey: message.pubkey,
    authorName: message.author,
    preview: message.body?.slice(0, PREVIEW_MAX_LENGTH),
    createdAt: message.createdAt,
    threadRootId: message.rootId ?? undefined,
  };
}
