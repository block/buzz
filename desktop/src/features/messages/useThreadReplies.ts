import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { mergeMessages } from "@/features/messages/hooks";
import { getThreadReplies } from "@/shared/api/tauri";
import type { Channel, RelayEvent, ThreadCursor } from "@/shared/api/types";

// Bounded per-page fetch; the hook pages to the floor so this is a page size,
// not a terminal cap. Matches the desktop command's own 500 max.
const THREAD_PAGE_LIMIT = 200;
// A hard stop so a pathological/looping cursor can never spin forever. At 200
// replies per page this covers a 100k-reply thread — far past any real one.
const MAX_THREAD_PAGES = 500;

/**
 * When a thread is open, fetch its full reply subtree server-side and merge the
 * events into the channel cache.
 *
 * The thread panel derives its replies from the channel cache
 * (`channelMessagesKey`); `useLoadMissingAncestors` only backfills *ancestors*
 * (walking `e`-tags upward), so replies that fell outside the channel
 * cold-load window were never fetched — the thread rendered silently
 * incomplete. This closes that descendant gap using the same cache seam: page
 * `get_thread_replies` to the floor (gap-free `(created_at, event_id)` keyset)
 * and merge each event in. All downstream grouping/ordering/unread derivation
 * keeps working unchanged; the thread simply becomes complete.
 *
 * Idempotent per (channel, root): `mergeMessages` dedupes by id, so replies
 * already in the cache from the live subscription or cold load are no-ops.
 */
export function useThreadReplies(
  activeChannel: Channel | null,
  openThreadRootId: string | null,
) {
  const queryClient = useQueryClient();
  const activeChannelId = activeChannel?.id ?? null;
  const activeChannelType = activeChannel?.channelType ?? null;
  // Track which roots we've already fetched per channel so re-opening a thread
  // (or a re-render) doesn't re-page the whole subtree every time.
  const fetchedRootsRef = React.useRef<Set<string>>(new Set());
  const previousChannelIdRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (previousChannelIdRef.current === activeChannelId) {
      return;
    }
    previousChannelIdRef.current = activeChannelId;
    fetchedRootsRef.current.clear();
  }, [activeChannelId]);

  React.useEffect(() => {
    if (
      !activeChannelId ||
      activeChannelType === "forum" ||
      !openThreadRootId
    ) {
      return;
    }
    if (fetchedRootsRef.current.has(openThreadRootId)) {
      return;
    }
    fetchedRootsRef.current.add(openThreadRootId);

    const channelId = activeChannelId;
    const rootId = openThreadRootId;
    let isCancelled = false;
    let completed = false;

    void (async () => {
      let cursor: ThreadCursor | null = null;
      try {
        for (let page = 0; page < MAX_THREAD_PAGES; page++) {
          const response = await getThreadReplies(rootId, channelId, {
            limit: THREAD_PAGE_LIMIT,
            cursor,
          });
          if (isCancelled) {
            return;
          }

          if (response.events.length > 0) {
            queryClient.setQueryData<RelayEvent[]>(
              channelMessagesKey(channelId),
              (current = []) => response.events.reduce(mergeMessages, current),
            );
          }

          if (!response.nextCursor) {
            completed = true;
            break;
          }
          cursor = response.nextCursor;
        }
      } catch (error) {
        // Let a later re-open retry rather than caching a partial subtree.
        fetchedRootsRef.current.delete(rootId);
        console.error("Failed to load thread replies", rootId, error);
      }
    })();

    return () => {
      isCancelled = true;
      if (!completed) {
        fetchedRootsRef.current.delete(rootId);
      }
    };
  }, [activeChannelId, activeChannelType, openThreadRootId, queryClient]);
}
