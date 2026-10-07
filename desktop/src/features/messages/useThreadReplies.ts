import {
  type QueryClient,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

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

export function collectThreadAuxMessageIds(
  threadRootId: string,
  replies: RelayEvent[],
): string[] {
  return [
    ...new Set([threadRootId, ...collectMessageIdsForAuxBackfill(replies)]),
  ];
}

async function withThreadAux(
  channelId: string,
  threadRootId: string,
  replies: RelayEvent[],
): Promise<RelayEvent[]> {
  const messageIds = collectThreadAuxMessageIds(threadRootId, replies);
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

async function loadThreadReplies(
  queryClient: QueryClient,
  channelId: string,
  rootId: string,
): Promise<RelayEvent[]> {
  const queryKey = threadRepliesKey(channelId, rootId);
  const cacheAtStart = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
  const idsAtStart = new Set(cacheAtStart.map((event) => event.id));
  const replies: RelayEvent[] = [];
  let cursor: ThreadCursor | null = null;
  for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
    const response = await getThreadReplies(rootId, channelId, {
      limit: THREAD_PAGE_LIMIT,
      cursor,
    });
    replies.push(...response.events);
    if (!response.nextCursor) {
      const fetched = await withThreadAux(channelId, rootId, replies);
      const current = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const receivedInFlight = current.filter(
        (event) => !idsAtStart.has(event.id),
      );
      return sortMessages([...fetched, ...receivedInFlight]);
    }
    cursor = response.nextCursor;
  }
  throw new Error(`Thread ${rootId} exceeded the page safety limit.`);
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
      return loadThreadReplies(queryClient, activeChannel.id, openThreadRootId);
    },
    staleTime: 0,
    gcTime: 60 * 60 * 1_000,
  });
}

/**
 * Load every summarized reply subtree for a channel-style Huddle transcript.
 * Ordinary channels keep replies in their thread panels; Huddles flatten those
 * replies into the chat timeline so companion and in-app presentations show the
 * same conversation without opening a transient thread surface.
 */
export function useThreadRepliesForRoots(
  activeChannel: Channel | null,
  rootIds: readonly string[],
) {
  const queryClient = useQueryClient();
  const channelId = activeChannel?.id ?? "none";
  return useQueries({
    queries: rootIds.map((rootId) => ({
      queryKey: threadRepliesKey(channelId, rootId),
      enabled: activeChannel !== null && activeChannel.channelType !== "forum",
      queryFn: () => loadThreadReplies(queryClient, channelId, rootId),
      staleTime: 0,
      gcTime: 60 * 60 * 1_000,
    })),
    combine: (results) => ({
      events: sortMessages(results.flatMap((result) => result.data ?? [])),
      isPending: results.some((result) => result.isPending),
    }),
  });
}
