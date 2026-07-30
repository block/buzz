import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { useStableSet } from "@/shared/hooks/useStableReference";
import {
  bookmarkedIdsFromStore,
  DEFAULT_STORE,
  mergeStores,
  PREVIEW_MAX_LENGTH,
  readBookmarkStore,
  savedEntriesFromStore,
  storageKey,
  writeBookmarkStore,
  type BookmarkEntry,
  type BookmarkStore,
} from "./bookmarksStorage";
import { BookmarkSyncManager } from "./bookmarksSync";
import type { RemoteBookmarks } from "./bookmarksSync";

/** Snapshot a caller supplies when saving a message. */
export type BookmarkTarget = {
  eventId: string;
  channelId: string;
  authorPubkey?: string;
  authorName?: string;
  preview?: string;
  createdAt?: number;
  /** Thread root id when the message is a reply — enables jump-to-context to
   *  open the thread it lives in, not just the channel. */
  threadRootId?: string;
};

export type UseBookmarks = {
  bookmarkedIds: ReadonlySet<string>;
  savedEntries: Array<{ eventId: string; entry: BookmarkEntry }>;
  isBookmarked: (eventId: string) => boolean;
  toggleBookmark: (target: BookmarkTarget) => void;
};

export function useBookmarks(pubkey: string | undefined): UseBookmarks {
  const [store, setStore] = React.useState<BookmarkStore>(() => {
    if (!pubkey) return DEFAULT_STORE;
    return readBookmarkStore(pubkey);
  });

  const managerRef = React.useRef<BookmarkSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readBookmarkStore(pubkey));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new BookmarkSyncManager(pubkey);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey]);

  React.useEffect(() => {
    if (!pubkey) return;
    const key = storageKey(pubkey);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) return;
      setStore(readBookmarkStore(pubkey));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey]);

  const applyRemote = React.useCallback(
    (remote: RemoteBookmarks): ((prev: BookmarkStore) => BookmarkStore) => {
      return (prev) => {
        if (!pubkey) return prev;
        if (remote.createdAt < lastAppliedRemoteTs.current) return prev;
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId <= lastAppliedEventId.current
        )
          return prev;
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        managerRef.current?.cancelPendingBookmarkPublish();
        const merged = mergeStores(prev, remote.store);
        if (!writeBookmarkStore(pubkey, merged)) return prev;
        return merged;
      };
    },
    [pubkey],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    void managerRef.current?.fetchRemoteBookmarks().then((remote) => {
      if (cancelled) return;
      if (remote) {
        setStore(applyRemote(remote));
      } else {
        const local = readBookmarkStore(pubkey);
        if (Object.keys(local.bookmarks).length > 0) {
          managerRef.current?.publishBookmarks(local);
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, applyRemote]);

  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribeToBookmarks((remote) => {
        if (cancelled) return;
        setStore(applyRemote(remote));
      })
      .then((dispose) => {
        if (cancelled) {
          void dispose();
        } else {
          unsub = dispose;
        }
      });
    return () => {
      cancelled = true;
      if (unsub) void unsub();
    };
  }, [pubkey, applyRemote]);

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    const unsub = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemoteBookmarks().then((remote) => {
        if (cancelled) return;
        if (remote) {
          setStore(applyRemote(remote));
        }
        const pending = managerRef.current?.getPendingBookmarkStore();
        if (pending) {
          managerRef.current?.publishBookmarks(pending);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: store.bookmarks is the relevant dep — the outer store identity can change without bookmarks changing (e.g., on reconnect writes)
  const bookmarkedIdsRaw = React.useMemo(
    () => bookmarkedIdsFromStore(store),
    [store.bookmarks],
  );
  // Content-stable: a no-op remote sync/reconnect rebuilds `store.bookmarks`
  // with a fresh identity but identical membership — `useStableSet` preserves
  // the previous Set so `isBookmarked` (and the actions context that every
  // message row consumes) does not churn the timeline. Genuine toggles change
  // membership and correctly produce a new identity.
  const bookmarkedIds = useStableSet(bookmarkedIdsRaw);

  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  const savedEntries = React.useMemo(
    () => savedEntriesFromStore(store),
    [store.bookmarks],
  );

  // Stable across toggles (dep is only `pubkey`); the current state is read
  // inside the functional updater so rapid double-clicks batch correctly.
  const toggleBookmark = React.useCallback(
    (target: BookmarkTarget) => {
      if (!pubkey) return;
      setStore((prev) => {
        const currently = prev.bookmarks[target.eventId]?.bookmarked === true;
        const entry: BookmarkEntry = {
          bookmarked: !currently,
          updatedAt: Math.floor(Date.now() / 1000),
          channelId: target.channelId,
          authorPubkey: target.authorPubkey,
          authorName: target.authorName,
          preview: target.preview?.slice(0, PREVIEW_MAX_LENGTH),
          createdAt: target.createdAt,
          threadRootId: target.threadRootId,
        };
        const next: BookmarkStore = {
          version: 1,
          bookmarks: { ...prev.bookmarks, [target.eventId]: entry },
        };
        if (!writeBookmarkStore(pubkey, next)) return prev;
        managerRef.current?.publishBookmarks(next);
        return next;
      });
    },
    [pubkey],
  );

  const isBookmarked = React.useCallback(
    (eventId: string) => bookmarkedIds.has(eventId),
    [bookmarkedIds],
  );

  return React.useMemo(
    () => ({ bookmarkedIds, savedEntries, isBookmarked, toggleBookmark }),
    [bookmarkedIds, savedEntries, isBookmarked, toggleBookmark],
  );
}
