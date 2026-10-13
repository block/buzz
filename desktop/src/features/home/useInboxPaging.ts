import * as React from "react";

import { getHomeFeed } from "@/shared/api/tauri";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";

/** Page size for older inbox pages (matches the first page). */
export const INBOX_PAGE_SIZE = 50;

/** Inbox paging is web-only: the web get_feed honours `until`; desktop's Rust command does not yet. */
export const INBOX_PAGING_ENABLED =
  import.meta.env?.VITE_BUZZ_PLATFORM === "web";

function oldestCreatedAt(feed: HomeFeedResponse["feed"]): number | null {
  let oldest: number | null = null;
  for (const item of [...feed.mentions, ...feed.needsAction]) {
    if (oldest === null || item.createdAt < oldest) oldest = item.createdAt;
  }
  return oldest;
}

function mergeUnique(base: FeedItem[], extra: FeedItem[]): FeedItem[] {
  if (extra.length === 0) return base;
  const seen = new Set(base.map((i) => i.id));
  return [...base, ...extra.filter((i) => !seen.has(i.id) && seen.add(i.id))];
}

/**
 * Appends older inbox pages below the live first page. The first page keeps polling as before;
 * older pages are fetched on demand with `until = oldest loaded createdAt`.
 */
export function useInboxPaging(firstPage: HomeFeedResponse | undefined) {
  const [older, setOlder] = React.useState<FeedItem[][]>([]);
  const [olderApprovals, setOlderApprovals] = React.useState<FeedItem[]>([]);
  const [hasMore, setHasMore] = React.useState(true);
  const [isLoadingOlder, setIsLoadingOlder] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const inFlight = React.useRef(false);

  const firstCount = firstPage
    ? firstPage.feed.mentions.length + firstPage.feed.needsAction.length
    : 0;
  // A short first page means there is nothing older.
  const firstPageFull = firstPage
    ? firstPage.feed.mentions.length >= INBOX_PAGE_SIZE
    : false;

  const merged = React.useMemo((): HomeFeedResponse | undefined => {
    if (!firstPage) return undefined;
    if (older.length === 0 && olderApprovals.length === 0) return firstPage;
    return {
      ...firstPage,
      feed: {
        ...firstPage.feed,
        mentions: mergeUnique(firstPage.feed.mentions, older.flat()),
        needsAction: mergeUnique(firstPage.feed.needsAction, olderApprovals),
      },
    };
  }, [firstPage, older, olderApprovals]);

  const loadOlder = React.useCallback(async () => {
    if (!INBOX_PAGING_ENABLED || !merged || inFlight.current) return;
    const until = oldestCreatedAt(merged.feed);
    if (until === null) {
      setHasMore(false);
      return;
    }
    inFlight.current = true;
    setIsLoadingOlder(true);
    setError(null);
    try {
      const page = await getHomeFeed({
        limit: INBOX_PAGE_SIZE,
        types: "mentions,needs_action",
        until,
      });
      setOlder((prev) => [...prev, page.feed.mentions]);
      setOlderApprovals((prev) => [...prev, ...page.feed.needsAction]);
      if (page.feed.mentions.length < INBOX_PAGE_SIZE) setHasMore(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't load older messages");
    } finally {
      inFlight.current = false;
      setIsLoadingOlder(false);
    }
  }, [merged]);

  return {
    feed: merged,
    hasMore: INBOX_PAGING_ENABLED && firstCount > 0 && firstPageFull && hasMore,
    isLoadingOlder,
    loadOlderError: error,
    loadOlder,
  };
}
