import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useAppShell } from "@/app/AppShellContext";
import { useActiveAgentTurnsByThread } from "@/features/agents/activeAgentTurnsStore";
import { getThreadReference } from "@/features/messages/lib/threading";
import {
  applyRootTitleWrite,
  applyTitleWrite,
  batchTitleSyncChannels,
  countUnreadByTitledRoot,
  createThreadTitleBatcher,
  describeTitleSaveError,
  summarizeWorkingTitledThreads,
  THREAD_TITLES_QUERY_KEY,
  threadTitleRootQueryKey,
  type WorkingTitledThreads,
} from "@/features/threads/lib/threadTitles";
import { relayClient } from "@/shared/api/relayClient";
import {
  getThreadTitles,
  listThreadTitles,
  setThreadTitle,
  type ThreadTitle,
  type ThreadTitleRecord,
} from "@/shared/api/tauriThreadTitles";
import { KIND_ARTIFACT, KIND_ARTIFACT_REMOVAL } from "@/shared/constants/kinds";

/** Titles live in other channels too; poll so the Threads view stays current. */
const THREAD_TITLES_REFRESH_MS = 60_000;
const LIVE_RETRY_MS = 5_000;
const THREAD_TITLE_STALE_MS = 30_000;

const loadThreadTitle = createThreadTitleBatcher(getThreadTitles);

/** Every titled thread the user can read, most recently active first. */
export function useThreadTitlesQuery(options?: { refetchInterval?: boolean }) {
  return useQuery({
    queryKey: THREAD_TITLES_QUERY_KEY,
    queryFn: listThreadTitles,
    staleTime: THREAD_TITLE_STALE_MS,
    refetchInterval: options?.refetchInterval
      ? THREAD_TITLES_REFRESH_MS
      : false,
  });
}

/**
 * One root's live title, looked up by root rather than read from the Threads
 * list, so it stays exact however many titles exist. Lookups made in the same
 * tick share one relay query.
 */
function useThreadTitleRecord(rootId: string | null) {
  const root = rootId?.toLowerCase() ?? null;
  const query = useQuery({
    queryKey: threadTitleRootQueryKey(root ?? ""),
    queryFn: () => loadThreadTitle(root ?? ""),
    enabled: root !== null,
    staleTime: THREAD_TITLE_STALE_MS,
  });
  return root === null ? null : (query.data ?? null);
}

export function useThreadTitle(
  channelId: string | null,
  rootId: string | null,
): ThreadTitleRecord | null {
  const title = useThreadTitleRecord(rootId);
  return channelId && title?.channelId === channelId ? title : null;
}

/**
 * Title for a thread root without knowing its channel; `null` skips the
 * lookup. Event ids are unique and the relay only accepts a title whose root
 * exists in its home channel, so the root alone identifies the thread.
 */
export function useThreadTitleForRoot(rootId: string | null): string | null {
  return useThreadTitleRecord(rootId)?.title ?? null;
}

/**
 * Unread reply counts per titled thread, keyed by root id. Uses the same
 * thread read state as the channel sidebar's unread dot, so the two agree.
 */
export function useTitledThreadUnreadCounts(): ReadonlyMap<string, number> {
  const query = useThreadTitlesQuery();
  const { unreadThreadFeedItems } = useAppShell();
  return React.useMemo(
    () =>
      countUnreadByTitledRoot(
        query.data,
        unreadThreadFeedItems.map((item) => ({
          id: item.id,
          rootId: getThreadReference(item.tags).rootId,
        })),
      ),
    [query.data, unreadThreadFeedItems],
  );
}

/** Agent work in threads the Threads view lists, for its sidebar entry. */
export function useWorkingTitledThreads(): WorkingTitledThreads | null {
  const query = useThreadTitlesQuery();
  const workingThreads = useActiveAgentTurnsByThread();
  return React.useMemo(
    () => summarizeWorkingTitledThreads(query.data, workingThreads),
    [query.data, workingThreads],
  );
}

/**
 * Reads a thread while it is open outside its channel, including replies that
 * arrive while it stays open. Mirrors Inbox: advance each reply's own marker
 * and the thread frontier, and drop any manual "mark unread" override.
 */
export function useMarkOpenThreadRead(rootId: string | null) {
  const {
    feedItemState,
    markMessageRead,
    markThreadRead,
    unreadThreadFeedItems,
  } = useAppShell();
  const unread = React.useMemo(
    () =>
      rootId
        ? unreadThreadFeedItems.filter(
            (item) => getThreadReference(item.tags).rootId === rootId,
          )
        : [],
    [rootId, unreadThreadFeedItems],
  );

  React.useEffect(() => {
    if (!rootId || unread.length === 0) return;
    let latest = 0;
    for (const item of unread) {
      feedItemState.undoUnread(item.id);
      markMessageRead(item.id, item.createdAt);
      latest = Math.max(latest, item.createdAt);
    }
    markThreadRead(rootId, latest);
  }, [
    feedItemState.undoUnread,
    markMessageRead,
    markThreadRead,
    rootId,
    unread,
  ]);
}

export function useSetThreadTitleMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setThreadTitle,
    onMutate: async (write) => {
      await queryClient.cancelQueries({ queryKey: THREAD_TITLES_QUERY_KEY });
      const rootKey = threadTitleRootQueryKey(write.rootId);
      const previous = queryClient.getQueryData<ThreadTitle[]>(
        THREAD_TITLES_QUERY_KEY,
      );
      const previousRecord = queryClient.getQueryData<ThreadTitleRecord | null>(
        rootKey,
      );
      const now = Math.floor(Date.now() / 1000);
      queryClient.setQueryData<ThreadTitle[]>(THREAD_TITLES_QUERY_KEY, (old) =>
        applyTitleWrite(old, write, now),
      );
      queryClient.setQueryData<ThreadTitleRecord | null>(rootKey, (old) =>
        applyRootTitleWrite(old, write, now),
      );
      return { previous, previousRecord };
    },
    onSuccess: (result, write) => {
      const stored = {
        ...write,
        title: result.title,
        revision: result.revision,
      };
      const now = Math.floor(Date.now() / 1000);
      queryClient.setQueryData<ThreadTitle[]>(THREAD_TITLES_QUERY_KEY, (old) =>
        applyTitleWrite(old, stored, now),
      );
      queryClient.setQueryData<ThreadTitleRecord | null>(
        threadTitleRootQueryKey(write.rootId),
        (old) => applyRootTitleWrite(old, stored, now),
      );
    },
    // Hook-level so the toast still shows when the editor unmounted on blur.
    onError: (error, write, context) => {
      queryClient.setQueryData(THREAD_TITLES_QUERY_KEY, context?.previous);
      queryClient.setQueryData(
        threadTitleRootQueryKey(write.rootId),
        context?.previousRecord,
      );
      toast.error(describeTitleSaveError(error));
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: THREAD_TITLES_QUERY_KEY });
    },
  });
}

/**
 * Refetch titles when anyone edits an artifact in these channels. Artifact
 * type can't be filtered on a live REQ, so any artifact change triggers a
 * refetch. The relay only fans channel events out to subscriptions that name
 * the channel, so a channel-less subscription would never fire.
 */
export function useThreadTitleLiveSync(channelIds: readonly string[]) {
  const queryClient = useQueryClient();
  const handleArtifactEvent = React.useEffectEvent(() => {
    void queryClient.invalidateQueries({ queryKey: THREAD_TITLES_QUERY_KEY });
  });
  const batchesKey = JSON.stringify(batchTitleSyncChannels(channelIds));

  React.useEffect(() => {
    const batches: string[][] = JSON.parse(batchesKey);
    if (batches.length === 0) {
      return;
    }
    let isCancelled = false;
    const disposers: Array<() => Promise<void>> = [];
    const retryTimers = new Set<ReturnType<typeof globalThis.setTimeout>>();

    const subscribe = (channels: string[]) => {
      relayClient
        .subscribeLive(
          {
            kinds: [KIND_ARTIFACT, KIND_ARTIFACT_REMOVAL],
            "#h": channels,
            limit: 0,
            since: Math.floor(Date.now() / 1000),
          },
          handleArtifactEvent,
        )
        .then((nextDispose) => {
          if (isCancelled) {
            void nextDispose();
            return;
          }
          disposers.push(nextDispose);
        })
        .catch((error: unknown) => {
          console.error("Failed to subscribe to thread title changes", error);
          if (!isCancelled) {
            const timer = globalThis.setTimeout(() => {
              retryTimers.delete(timer);
              subscribe(channels);
            }, LIVE_RETRY_MS);
            retryTimers.add(timer);
          }
        });
    };
    for (const channels of batches) {
      subscribe(channels);
    }
    // A title set while the socket was down never reaches the live REQ.
    const unsubscribeReconnects =
      relayClient.subscribeToReconnects(handleArtifactEvent);

    return () => {
      isCancelled = true;
      for (const timer of retryTimers) {
        globalThis.clearTimeout(timer);
      }
      unsubscribeReconnects();
      for (const dispose of disposers) {
        void dispose();
      }
    };
  }, [batchesKey]);
}
