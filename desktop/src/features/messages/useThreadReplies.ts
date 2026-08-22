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

// The bridge clamps a thread page to BRIDGE_THREAD_MAX_LIMIT (500) and the
// Tauri command caps it with `.min(500)`, so 500 is the largest page the server
// will serve. A thread with fewer than 500 replies cold-opens in a single
// content round trip; a full 500-reply page returns a non-null cursor (the
// bridge treats every full page as potentially incomplete), so exactly 500
// costs a second, empty content request.
export const THREAD_PAGE_LIMIT = 500;
const MAX_THREAD_PAGES = 500;

// Reopening a recently-loaded thread within this window is a cache hit rather
// than a full refetch. While the panel's channel is active the live WS
// subscription writes every new content and aux event into the thread-replies
// key (`hooks.ts` `appendMessage`), so a warm cache stays current on its own.
// The bound exists because that self-healing only covers the *subscribed*
// channel: edits/deletions/reactions that arrive for a thread whose channel is
// not currently subscribed never reach the cache, so a finite staleTime
// guarantees the next mount refetches and corrects them. 30s is short enough to
// self-heal quickly yet long enough to collapse the open/close/reopen storm.
export const THREAD_REPLIES_STALE_TIME_MS = 30_000;

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

/**
 * Aux fetchers for {@link backfillThreadAux}, injectable so the merge/degrade
 * behavior is unit-testable without a relay. Defaults hit the live relay.
 */
export type ThreadAuxFetchDeps = {
  fetchStructuralAux: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
  fetchReactions: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
};

const defaultThreadAuxDeps: ThreadAuxFetchDeps = {
  fetchStructuralAux: fetchStructuralAuxForMessages,
  fetchReactions: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelReactionAuxFilter,
    ),
};

/**
 * Hydrate the structural aux closure (edits/deletions) and reactions for a set
 * of replies into the thread-replies cache, mirroring the channel timeline's
 * post-history backfill (`backfillAuxForMessages`). Kept off the first-paint
 * critical path: `loadThreadReplies` resolves with bare content and fires this
 * without awaiting, so an edited reply may briefly render its original text
 * until the merge lands (the same accepted tradeoff the timeline ships).
 *
 * Both branches are best-effort — a failing aux fetch degrades to no adornment
 * rather than corrupting the cache. The merge writes through a functional
 * updater keyed on the thread's `(channelId, rootId)`, so it can only touch
 * that thread's cache and never resurrects content: dedupe-by-id folds the aux
 * into whatever content is current, and aux referencing a message a later
 * refetch dropped simply renders against nothing.
 */
export async function backfillThreadAux(
  queryClient: QueryClient,
  channelId: string,
  rootId: string,
  replies: RelayEvent[],
  deps: ThreadAuxFetchDeps = defaultThreadAuxDeps,
): Promise<void> {
  const messageIds = collectThreadAuxMessageIds(rootId, replies);
  const [structuralAux, reactions] = await Promise.all([
    fetchThreadAuxBestEffort("structural aux", channelId, () =>
      deps.fetchStructuralAux(channelId, messageIds),
    ),
    fetchThreadAuxBestEffort("reactions", channelId, () =>
      deps.fetchReactions(channelId, messageIds),
    ),
  ]);
  const auxEvents = [...structuralAux, ...reactions];
  if (auxEvents.length === 0) {
    return;
  }
  queryClient.setQueryData<RelayEvent[]>(
    threadRepliesKey(channelId, rootId),
    (current = []) => sortMessages([...current, ...auxEvents]),
  );
}

/**
 * Dependencies for {@link loadThreadReplies}, injectable so the first-paint
 * seam (content resolves before aux settles; aux later merges into the same
 * cache key) is unit-testable while stubbing only the relay boundary. Defaults
 * hit the live relay.
 */
export type LoadThreadRepliesDeps = {
  fetchPage: typeof getThreadReplies;
  auxDeps: ThreadAuxFetchDeps;
};

const defaultLoadThreadRepliesDeps: LoadThreadRepliesDeps = {
  fetchPage: getThreadReplies,
  auxDeps: defaultThreadAuxDeps,
};

export async function loadThreadReplies(
  queryClient: QueryClient,
  channelId: string,
  rootId: string,
  deps: LoadThreadRepliesDeps = defaultLoadThreadRepliesDeps,
): Promise<RelayEvent[]> {
  const queryKey = threadRepliesKey(channelId, rootId);
  const cacheAtStart = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
  const idsAtStart = new Set(cacheAtStart.map((event) => event.id));
  const replies: RelayEvent[] = [];
  let cursor: ThreadCursor | null = null;
  for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
    const response = await deps.fetchPage(rootId, channelId, {
      limit: THREAD_PAGE_LIMIT,
      cursor,
    });
    replies.push(...response.events);
    if (!response.nextCursor) {
      // Resolve with content now; hydrate aux into the cache off the critical
      // path. The aux fetch is a relay round trip, so React Query commits this
      // returned content before `backfillThreadAux`'s merge runs — the merge
      // then folds aux over the committed rows via `setQueryData`.
      void backfillThreadAux(
        queryClient,
        channelId,
        rootId,
        replies,
        deps.auxDeps,
      );
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
    staleTime: THREAD_REPLIES_STALE_TIME_MS,
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
      staleTime: THREAD_REPLIES_STALE_TIME_MS,
      gcTime: 60 * 60 * 1_000,
    })),
    combine: (results) => ({
      events: sortMessages(results.flatMap((result) => result.data ?? [])),
      isPending: results.some((result) => result.isPending),
    }),
  });
}
