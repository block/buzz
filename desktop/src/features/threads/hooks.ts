import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  applyRootTitleWrite,
  batchTitleSyncChannels,
  createThreadTitleBatcher,
  describeTitleSaveError,
  THREAD_TITLES_QUERY_KEY,
  threadTitleRootQueryKey,
} from "@/features/threads/lib/threadTitles";
import { relayClient } from "@/shared/api/relayClient";
import {
  getThreadTitles,
  setThreadTitle,
  type ThreadTitleRecord,
} from "@/shared/api/tauriThreadTitles";
import { KIND_ARTIFACT, KIND_ARTIFACT_REMOVAL } from "@/shared/constants/kinds";

const LIVE_RETRY_MS = 5_000;
const THREAD_TITLE_STALE_MS = 30_000;

const loadThreadTitle = createThreadTitleBatcher(getThreadTitles);

/**
 * One root's live title, looked up by its derived id, so it stays exact
 * however many titles exist. Lookups made in the same tick share one relay
 * query.
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

export function useSetThreadTitleMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: setThreadTitle,
    onMutate: async (write) => {
      const rootKey = threadTitleRootQueryKey(write.rootId);
      await queryClient.cancelQueries({ queryKey: rootKey });
      const previousRecord = queryClient.getQueryData<ThreadTitleRecord | null>(
        rootKey,
      );
      queryClient.setQueryData<ThreadTitleRecord | null>(rootKey, (old) =>
        applyRootTitleWrite(old, write, Math.floor(Date.now() / 1000)),
      );
      return { previousRecord };
    },
    onSuccess: (result, write) => {
      const stored = {
        ...write,
        title: result.title,
        revision: result.revision,
      };
      queryClient.setQueryData<ThreadTitleRecord | null>(
        threadTitleRootQueryKey(write.rootId),
        (old) =>
          applyRootTitleWrite(old, stored, Math.floor(Date.now() / 1000)),
      );
    },
    // Hook-level so the toast still shows when the editor unmounted on blur.
    onError: (error, write, context) => {
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
