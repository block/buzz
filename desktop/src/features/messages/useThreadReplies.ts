import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  collectMessageIdsForAuxBackfill,
  fetchStructuralAuxForMessages,
} from "@/features/messages/lib/auxBackfill";
import {
  threadRepliesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters";
import { getThreadReplies } from "@/shared/api/tauri";
import type { Channel, RelayEvent, ThreadCursor } from "@/shared/api/types";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 500;

/**
 * Append the structural aux closure (edits/deletions) for the fetched replies.
 * The server thread-subtree query resolves deletions itself but omits
 * kind:40003 edits, so a bare refetch would render every edited reply with its
 * original text. Best-effort: an aux failure logs and returns the replies
 * unadorned rather than failing the whole thread load.
 */
async function fetchThreadAuxBestEffort(
  label: string,
  channelId: string,
  fetchAux: () => Promise<RelayEvent[]>,
): Promise<RelayEvent[]> {
  try {
    return await fetchAux();
  } catch (error) {
    console.error(
      `Failed to backfill thread reply ${label} for channel`,
      channelId,
      error,
    );
    return [];
  }
}

async function withThreadAux(
  channelId: string,
  replies: RelayEvent[],
): Promise<RelayEvent[]> {
  const messageIds = collectMessageIdsForAuxBackfill(replies);
  const [structuralAux, reactions] = await Promise.all([
    fetchThreadAuxBestEffort("structural aux", channelId, () =>
      fetchStructuralAuxForMessages(channelId, messageIds),
    ),
    fetchThreadAuxBestEffort("reactions", channelId, () =>
      relayClient.fetchAuxEventsByReference(
        channelId,
        messageIds,
        buildChannelReactionAuxFilter,
      ),
    ),
  ]);
  return sortMessages([...replies, ...structuralAux, ...reactions]);
}

/** Fetch a thread subtree into a cache independent from channel window pages. */
export function useThreadReplies(
  activeChannel: Channel | null,
  openThreadRootId: string | null,
) {
  const channelId = activeChannel?.id ?? "none";
  const rootId = openThreadRootId ?? "none";
  const queryClient = useQueryClient();
  const queryKey = threadRepliesKey(channelId, rootId);
  return useQuery({
    queryKey,
    enabled:
      activeChannel !== null &&
      activeChannel.channelType !== "forum" &&
      openThreadRootId !== null,
    queryFn: async (): Promise<RelayEvent[]> => {
      if (!activeChannel || !openThreadRootId) return [];
      const cacheAtStart =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const idsAtStart = new Set(cacheAtStart.map((event) => event.id));
      const replies: RelayEvent[] = [];
      let cursor: ThreadCursor | null = null;
      for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
        const response = await getThreadReplies(
          openThreadRootId,
          activeChannel.id,
          { limit: THREAD_PAGE_LIMIT, cursor },
        );
        replies.push(...response.events);
        if (!response.nextCursor) {
          const fetched = await withThreadAux(activeChannel.id, replies);
          const current =
            queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
          const receivedInFlight = current.filter(
            (event) => !idsAtStart.has(event.id),
          );
          return sortMessages([...fetched, ...receivedInFlight]);
        }
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
