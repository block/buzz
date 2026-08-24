import {
  type QueryClient,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  threadRepliesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { getThreadReplies } from "@/shared/api/tauri";
import type { Channel, RelayEvent, ThreadCursor } from "@/shared/api/types";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 500;

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
      const current = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const receivedInFlight = current.filter(
        (event) => !idsAtStart.has(event.id),
      );
      return sortMessages([...replies, ...receivedInFlight]);
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
 * Aggregate a set of per-root thread-reply query results into one view for a
 * multi-root consumer. Pure over the results array so the load-bearing
 * error-surfacing contract is unit-testable without a live QueryClient.
 *
 * `isError`/`error` expose aggregate terminal failure so a consumer never
 * silently drops a failed reply subtree — the same false-empty class the
 * single-root panel guards against. `error` carries the first failed subtree's
 * error; `refetch` re-runs only the failed queries so a partial success is not
 * needlessly re-fetched.
 */
export function combineThreadRepliesResults(
  results: readonly {
    data?: RelayEvent[];
    isPending: boolean;
    isError: boolean;
    error: unknown;
    refetch: () => unknown;
  }[],
) {
  return {
    events: sortMessages(results.flatMap((result) => result.data ?? [])),
    isPending: results.some((result) => result.isPending),
    isError: results.some((result) => result.isError),
    error: results.find((result) => result.isError)?.error ?? null,
    refetch: () => {
      for (const result of results) {
        if (result.isError) void result.refetch();
      }
    },
  };
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
    combine: combineThreadRepliesResults,
  });
}
