import { useQuery } from "@tanstack/react-query";

import { threadRepliesKey } from "@/features/messages/lib/messageQueryKeys";
import { getThreadReplies } from "@/shared/api/tauri";
import type { Channel, RelayEvent, ThreadCursor } from "@/shared/api/types";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 500;

/** Fetch a thread subtree into a cache independent from channel window pages. */
export function useThreadReplies(
  activeChannel: Channel | null,
  openThreadRootId: string | null,
) {
  const channelId = activeChannel?.id ?? "none";
  const rootId = openThreadRootId ?? "none";
  return useQuery({
    queryKey: threadRepliesKey(channelId, rootId),
    enabled:
      activeChannel !== null &&
      activeChannel.channelType !== "forum" &&
      openThreadRootId !== null,
    queryFn: async (): Promise<RelayEvent[]> => {
      if (!activeChannel || !openThreadRootId) return [];
      const replies: RelayEvent[] = [];
      let cursor: ThreadCursor | null = null;
      for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
        const response = await getThreadReplies(
          openThreadRootId,
          activeChannel.id,
          { limit: THREAD_PAGE_LIMIT, cursor },
        );
        replies.push(...response.events);
        if (!response.nextCursor) return replies;
        cursor = response.nextCursor;
      }
      throw new Error(
        `Thread ${openThreadRootId} exceeded the page safety limit.`,
      );
    },
    staleTime: 0,
    gcTime: 60 * 60 * 1_000,
  });
}
