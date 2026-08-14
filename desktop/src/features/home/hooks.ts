import * as React from "react";

import { useQuery } from "@tanstack/react-query";

import { getOldestConversationActivity } from "@/features/home/lib/inbox";
import { getHomeFeed } from "@/shared/api/tauri";
import type { Channel, FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

/** Keeps focused polling at the established 30-second cadence. */
export const HOME_FEED_REFETCH_INTERVAL_MS = 30_000;
/** Suppresses the expensive focus refetch until the home feed is old. */
export const HOME_FEED_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy for the home feed query; consumed by focusRefetchPolicy.test.mjs. */
export const homeFeedFocusRefetchPolicy = {
  staleTime: HOME_FEED_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;

export function useHomeFeedQuery() {
  const connectionState = useRelayConnection();
  const connected = connectionState === "connected";
  const refetchInterval = useFocusedRefetchInterval(
    connected ? HOME_FEED_REFETCH_INTERVAL_MS : false,
  );

  return useQuery({
    queryKey: ["home-feed"],
    queryFn: () =>
      getHomeFeed({
        limit: 50,
        types: "mentions,needs_action,activity,agent_activity",
      }),
    gcTime: 5 * 60 * 1_000,
    // Pause background polling on degraded/stalled/disconnected connections.
    // The relay can't serve the request anyway, and the spurious failures
    // consume quota that the recovery path needs.
    refetchInterval,
    ...homeFeedFocusRefetchPolicy,
  });
}

/** Page size for older inbox pages — matches the first page's window. */
export const HOME_FEED_OLDER_PAGE_LIMIT = 50;

type HomeFeedPagination = {
  /** The first page merged with every older page fetched so far. */
  feed: HomeFeedResponse | undefined;
  /** Fetch the next page of older conversations. No-op while one is in flight. */
  fetchOlder: () => void;
  isFetchingOlder: boolean;
  /** False once a fetch returns nothing new — the feed window is exhausted. */
  hasOlder: boolean;
};

/**
 * Cursor pagination for the inbox, layered over the polling first page.
 *
 * The first page (from [`useHomeFeedQuery`]) stays live — re-polled every
 * 30s — while older pages are fetched once on demand and held locally.
 * The cursor is the oldest conversation's latest activity across the mention
 * events in hand (see `getOldestConversationActivity`); the relay returns the
 * next `HOME_FEED_OLDER_PAGE_LIMIT`-event window of whole conversations at or
 * older than it. The inclusive boundary conversation and any conversation
 * that later re-enters the live first page arrive as duplicate event ids, so
 * the merge dedupes by id; `buildInboxItems` then groups by conversation as
 * usual.
 *
 * Older pages request `types: "mentions"` only: the needs-action query is a
 * fixed-window side feed and `activity`/`agent_activity` are not served by
 * the native `get_feed` — mentions are what the inbox list is made of.
 *
 * Pagination is gated on the channel list being loaded: the cursor derivation
 * mirrors the relay's DM conversation key (`dm:<channel>`), which requires
 * channel types.
 */
export function useHomeFeedPagination(
  baseFeed: HomeFeedResponse | undefined,
  channels: Channel[] | undefined,
): HomeFeedPagination {
  const [olderMentions, setOlderMentions] = React.useState<FeedItem[]>([]);
  const [exhausted, setExhausted] = React.useState(false);
  const [isFetchingOlder, setIsFetchingOlder] = React.useState(false);
  const inFlightRef = React.useRef(false);

  const baseMentions = baseFeed?.feed.mentions;

  const feed = React.useMemo((): HomeFeedResponse | undefined => {
    if (!baseFeed) return undefined;
    if (olderMentions.length === 0) return baseFeed;

    const seen = new Set(baseFeed.feed.mentions.map((item) => item.id));
    const older = olderMentions.filter((item) => {
      if (seen.has(item.id)) return false;
      seen.add(item.id);
      return true;
    });

    return {
      ...baseFeed,
      feed: {
        ...baseFeed.feed,
        mentions: [...baseFeed.feed.mentions, ...older],
      },
    };
  }, [baseFeed, olderMentions]);

  const fetchOlder = React.useCallback(() => {
    if (inFlightRef.current || exhausted) return;
    // Without channel types the cursor cannot mirror the relay's DM
    // conversation key; paginating now could skip conversations. Wait.
    if (!baseMentions || !channels) return;

    const cursor = getOldestConversationActivity(
      [...baseMentions, ...olderMentions],
      channels,
    );
    if (cursor === null) return;

    inFlightRef.current = true;
    setIsFetchingOlder(true);
    void getHomeFeed({
      until: cursor,
      limit: HOME_FEED_OLDER_PAGE_LIMIT,
      types: "mentions",
    })
      .then((response) => {
        const known = new Set([
          ...baseMentions.map((item) => item.id),
          ...olderMentions.map((item) => item.id),
        ]);
        const fresh = response.feed.mentions.filter(
          (item) => !known.has(item.id),
        );
        if (fresh.length === 0) {
          setExhausted(true);
          return;
        }
        setOlderMentions((previous) => [...previous, ...fresh]);
      })
      .catch(() => {
        // Leave state unchanged: the next scroll-end retries.
      })
      .finally(() => {
        inFlightRef.current = false;
        setIsFetchingOlder(false);
      });
  }, [baseMentions, channels, exhausted, olderMentions]);

  return { feed, fetchOlder, isFetchingOlder, hasOlder: !exhausted };
}
