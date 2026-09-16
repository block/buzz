import {
  type QueryClient,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import * as React from "react";

import {
  threadRepliesKey,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { getThreadReplies } from "@/shared/api/tauri";
import type {
  Channel,
  RelayEvent,
  ThreadCursor,
  ThreadRepliesResponse,
} from "@/shared/api/types";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 500;

/** Error thrown when a paged fetch completed but a caller-expected event was absent. */
export class ThreadExpectedEventMissingError extends Error {
  readonly expectedEventId: string;
  constructor(expectedEventId: string) {
    super(
      `Thread fetch completed but expected event ${expectedEventId} is absent.`,
    );
    this.name = "ThreadExpectedEventMissingError";
    this.expectedEventId = expectedEventId;
  }
}

/** Shape of the per-page fetcher — matches `getThreadReplies` exactly. */
export type ThreadRepliesFetcher = (
  rootId: string,
  channelId: string,
  options: { limit: number; cursor: ThreadCursor | null },
) => Promise<ThreadRepliesResponse>;

export async function loadThreadReplies(
  queryClient: QueryClient,
  channelId: string,
  rootId: string,
  expectedEventId?: string | null,
  exhaustedTargets?: Set<string>,
  fetcher: ThreadRepliesFetcher = getThreadReplies,
): Promise<RelayEvent[]> {
  const queryKey = threadRepliesKey(channelId, rootId);
  const cacheAtStart = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
  const idsAtStart = new Set(cacheAtStart.map((event) => event.id));
  const replies: RelayEvent[] = [];
  let cursor: ThreadCursor | null = null;
  for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
    const response = await fetcher(rootId, channelId, {
      limit: THREAD_PAGE_LIMIT,
      cursor,
    });
    replies.push(...response.events);
    if (!response.nextCursor) {
      const current = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const receivedInFlight = current.filter(
        (event) => !idsAtStart.has(event.id),
      );
      const result = sortMessages([...replies, ...receivedInFlight]);
      // When the caller expects a specific reply event (e.g. opened from a
      // notification) and the completed fetch does not contain it, the relay
      // likely delivered an empty result before the event was replicated. Throw
      // so React Query's retry-with-backoff re-attempts instead of caching an
      // authoritative empty — heals automatically once the relay catches up.
      //
      // Once the target has been declared permanently absent (exhaustedTargets
      // contains it), stop throwing: the successfully fetched replies are
      // rendered and the unreachable target is quietly retired rather than
      // painting the whole thread as a terminal load error.
      if (
        expectedEventId &&
        !result.some((e) => e.id === expectedEventId) &&
        !exhaustedTargets?.has(expectedEventId)
      ) {
        throw new ThreadExpectedEventMissingError(expectedEventId);
      }
      return result;
    }
    cursor = response.nextCursor;
  }
  throw new Error(`Thread ${rootId} exceeded the page safety limit.`);
}

/** Fetch a thread subtree into a cache independent from channel window pages. */
export function useThreadReplies(
  activeChannel: Channel | null,
  openThreadRootId: string | null,
  expectedEventId?: string | null,
) {
  const channelId = activeChannel?.id ?? "none";
  const rootId = openThreadRootId ?? "none";
  const queryClient = useQueryClient();
  const queryKey = threadRepliesKey(channelId, rootId);

  // Tracks expected-event IDs that have permanently failed validation so that
  // a deleted/moderated target doesn't lock the thread in a terminal error
  // state. On the terminal attempt the query-fn writes the target here; the
  // next call to loadThreadReplies sees it and returns the fetched replies
  // directly instead of throwing.
  const exhaustedTargetsRef = React.useRef(new Set<string>());

  // Counts consecutive fetch attempts for the current expectedEventId.
  // When the count reaches the exhaustion threshold the query-fn declares
  // the target permanently absent before calling loadThreadReplies so the
  // terminal attempt resolves to data rather than throwing.
  const attemptCountRef = React.useRef<{ id: string; count: number } | null>(
    null,
  );

  // useQuery is declared before the target-change invalidation effect so that
  // TanStack's internal options-update effect (useBaseQuery.ts, registered
  // inside useQuery) runs first on each render cycle. React runs effects in
  // declaration order; placing our invalidation effect after useQuery ensures
  // the observer has installed the new queryFn closure — capturing the current
  // expectedEventId — before invalidateQueries triggers query.fetch(). Without
  // this ordering, a settled null→target transition refetches with the previous
  // (null) queryFn, loadThreadReplies validates against null, and the missing
  // reply stays absent with no Retry error surface.
  const queryResult = useQuery({
    queryKey,
    enabled:
      activeChannel !== null &&
      activeChannel.channelType !== "forum" &&
      openThreadRootId !== null,
    queryFn: async (): Promise<RelayEvent[]> => {
      if (!activeChannel || !openThreadRootId) return [];
      if (expectedEventId) {
        // Reset the counter when the target changes.
        if (attemptCountRef.current?.id !== expectedEventId) {
          attemptCountRef.current = { id: expectedEventId, count: 0 };
        }
        attemptCountRef.current.count += 1;
        // On the terminal attempt, declare the target permanently absent
        // before fetching so loadThreadReplies returns the fetched replies
        // directly instead of throwing. This ensures the query resolves to
        // success rather than landing in terminal error state — no re-entrant
        // scheduling is needed because the resolution happens synchronously
        // inside the query function itself.
        if (attemptCountRef.current.count >= 3) {
          exhaustedTargetsRef.current.add(expectedEventId);
        }
      }
      return loadThreadReplies(
        queryClient,
        activeChannel.id,
        openThreadRootId,
        expectedEventId,
        exhaustedTargetsRef.current,
      );
    },
    staleTime: 0,
    gcTime: 60 * 60 * 1_000,
    // Override global defaults: a missing reply should retry rather than
    // being permanently hidden. The ThreadExpectedEventMissingError path
    // throws on attempts 1 and 2; attempt 3 records exhaustion first so
    // loadThreadReplies returns data directly — the terminal attempt always
    // resolves to success.
    retry: 3,
    retryDelay: (attempt) => Math.min(1_000 * 2 ** attempt, 30_000),
  });

  // Notification routing can change expectedEventId while the same thread root
  // is already mounted and the query is settled. Because the query key
  // (channelId + rootId) stays the same, the query fn closure does not re-run
  // automatically. Invalidate explicitly so the new target gets a fresh fetch
  // and validation pass.
  //
  // This effect is declared AFTER useQuery so that TanStack's options-update
  // effect (which installs the new queryFn closure) always runs first within
  // the same render cycle. The invalidation therefore reaches query.fetch()
  // with the current expectedEventId already captured — not the previous one.
  //
  // If the query is still in-flight with no data yet (cold start race: target
  // arrives before the first page returns), cancel the in-flight fetch first.
  // Without the cancel the obsolete empty-page response can settle before the
  // new target's validation closure is active, producing a false authoritative
  // [] result.
  const prevExpectedEventIdRef = React.useRef(expectedEventId);
  React.useEffect(() => {
    const prev = prevExpectedEventIdRef.current;
    prevExpectedEventIdRef.current = expectedEventId;
    if (
      expectedEventId !== null &&
      expectedEventId !== undefined &&
      expectedEventId !== prev &&
      !exhaustedTargetsRef.current.has(expectedEventId)
    ) {
      const state = queryClient.getQueryState(queryKey);
      if (state?.fetchStatus === "fetching" && state.status === "pending") {
        // Cold fetch in-flight: cancel and re-fetch atomically so the stale
        // empty-page response cannot settle as authoritative data before the
        // new target's validation closure is active.
        void queryClient.cancelQueries({ queryKey }).then(() => {
          void queryClient.invalidateQueries({ queryKey });
        });
      } else {
        void queryClient.invalidateQueries({ queryKey });
      }
    }
  }, [expectedEventId, queryClient, queryKey]);

  return queryResult;
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
