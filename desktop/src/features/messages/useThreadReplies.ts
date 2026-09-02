import * as React from "react";

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
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { getThreadReplies } from "@/shared/api/tauri";
import type { Channel, RelayEvent, ThreadCursor } from "@/shared/api/types";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 500;
const EMPTY_THREAD_ROOT_IDS: ReadonlySet<string> = new Set();

export type InlineThreadController = {
  errorRootIds: ReadonlySet<string>;
  fetchingRootIds: ReadonlySet<string>;
  pendingRootIds: ReadonlySet<string>;
  rootIds: ReadonlySet<string>;
  onRetry: () => void;
  onToggle: (message: TimelineMessage) => void;
};

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

type ThreadReplyQueryResult = {
  data?: RelayEvent[];
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => unknown;
};

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
  results: readonly ThreadReplyQueryResult[],
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

export function combineThreadRepliesForRoots(
  rootIds: readonly string[],
  results: readonly ThreadReplyQueryResult[],
) {
  return {
    ...combineThreadRepliesResults(results),
    errorRootIds: new Set(
      rootIds.filter((_rootId, index) => results[index]?.isError),
    ),
    fetchingRootIds: new Set(
      rootIds.filter((_rootId, index) => results[index]?.isFetching),
    ),
    pendingRootIds: new Set(
      rootIds.filter((_rootId, index) => results[index]?.isPending),
    ),
  };
}

/** Load multiple reply subtrees into independently cached per-root queries. */
export function useThreadRepliesForRoots(
  activeChannel: Channel | null,
  rootIds: readonly string[],
) {
  const queryClient = useQueryClient();
  const channelId = activeChannel?.id ?? "none";
  const combine = React.useCallback(
    (results: readonly ThreadReplyQueryResult[]) =>
      combineThreadRepliesForRoots(rootIds, results),
    [rootIds],
  );
  return useQueries({
    queries: rootIds.map((rootId) => ({
      queryKey: threadRepliesKey(channelId, rootId),
      enabled: activeChannel !== null && activeChannel.channelType !== "forum",
      queryFn: () => loadThreadReplies(queryClient, channelId, rootId),
      staleTime: 0,
      gcTime: 60 * 60 * 1_000,
    })),
    combine,
  });
}

/** Controls on-demand reply subtrees rendered inside the main conversation. */
export function useInlineThreadReplies(activeChannel: Channel | null): {
  controller: InlineThreadController;
  events: RelayEvent[];
} {
  const activeChannelId = activeChannel?.id ?? null;
  const [state, setState] = React.useState<{
    channelId: string | null;
    rootIds: Set<string>;
  }>(() => ({ channelId: null, rootIds: new Set() }));
  const rootIds =
    state.channelId === activeChannelId ? state.rootIds : EMPTY_THREAD_ROOT_IDS;
  const rootIdList = React.useMemo(() => [...rootIds], [rootIds]);
  const replies = useThreadRepliesForRoots(activeChannel, rootIdList);
  const onToggle = React.useCallback(
    (message: TimelineMessage) => {
      if (!activeChannelId) return;
      setState((current) => {
        const nextRootIds = new Set(
          current.channelId === activeChannelId ? current.rootIds : [],
        );
        if (nextRootIds.has(message.id)) {
          nextRootIds.delete(message.id);
        } else {
          nextRootIds.add(message.id);
        }
        return { channelId: activeChannelId, rootIds: nextRootIds };
      });
    },
    [activeChannelId],
  );
  const onRetry = React.useCallback(() => replies.refetch(), [replies.refetch]);
  const controller = React.useMemo<InlineThreadController>(
    () => ({
      errorRootIds: replies.errorRootIds,
      fetchingRootIds: replies.fetchingRootIds,
      onRetry,
      onToggle,
      pendingRootIds: replies.pendingRootIds,
      rootIds,
    }),
    [
      onRetry,
      onToggle,
      replies.errorRootIds,
      replies.fetchingRootIds,
      replies.pendingRootIds,
      rootIds,
    ],
  );

  return React.useMemo(
    () => ({ controller, events: replies.events }),
    [controller, replies.events],
  );
}

export function collectNewlyRevealedInlineReplyIds({
  rootIds,
  pendingRootIds,
  fetchingRootIds,
  errorRootIds,
  events,
  revealedRootIds,
}: {
  rootIds: ReadonlySet<string>;
  pendingRootIds: ReadonlySet<string>;
  fetchingRootIds: ReadonlySet<string>;
  errorRootIds: ReadonlySet<string>;
  events: readonly RelayEvent[];
  revealedRootIds: ReadonlySet<string>;
}) {
  const activeRevealedRootIds = new Set(
    [...revealedRootIds].filter((rootId) => rootIds.has(rootId)),
  );
  const replyIdsByRoot = new Map<string, string[]>();
  for (const event of events) {
    const rootId = getThreadReference(event.tags).rootId;
    if (!rootId || !rootIds.has(rootId)) continue;
    const ids = replyIdsByRoot.get(rootId);
    if (ids) ids.push(event.id);
    else replyIdsByRoot.set(rootId, [event.id]);
  }

  const messageIds: string[] = [];
  for (const rootId of rootIds) {
    if (
      activeRevealedRootIds.has(rootId) ||
      pendingRootIds.has(rootId) ||
      fetchingRootIds.has(rootId)
    )
      continue;
    messageIds.push(rootId, ...(replyIdsByRoot.get(rootId) ?? []));
    if (!errorRootIds.has(rootId)) activeRevealedRootIds.add(rootId);
  }
  return { messageIds, revealedRootIds: activeRevealedRootIds };
}

export function useMarkInlineRepliesRead(
  inlineReplies: {
    controller: InlineThreadController;
    events: RelayEvent[];
  },
  markRevealedRepliesRead: (messageId: string) => void,
) {
  const revealedRootIdsRef = React.useRef<ReadonlySet<string>>(new Set());
  React.useEffect(() => {
    const revealed = collectNewlyRevealedInlineReplyIds({
      rootIds: inlineReplies.controller.rootIds,
      fetchingRootIds: inlineReplies.controller.fetchingRootIds,
      pendingRootIds: inlineReplies.controller.pendingRootIds,
      errorRootIds: inlineReplies.controller.errorRootIds,
      events: inlineReplies.events,
      revealedRootIds: revealedRootIdsRef.current,
    });
    revealedRootIdsRef.current = revealed.revealedRootIds;
    for (const messageId of revealed.messageIds) {
      markRevealedRepliesRead(messageId);
    }
  }, [
    inlineReplies.controller.fetchingRootIds,
    inlineReplies.controller.errorRootIds,
    inlineReplies.controller.pendingRootIds,
    inlineReplies.controller.rootIds,
    inlineReplies.events,
    markRevealedRepliesRead,
  ]);
}
