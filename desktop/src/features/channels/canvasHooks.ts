import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import { getCanvas, getCanvasHistory, setCanvas } from "@/shared/api/tauri";
import type {
  CanvasHistoryCursor,
  CanvasHistoryResponse,
} from "@/shared/api/types";

export function useCanvasQuery(channelId: string | null, enabled = true) {
  return useQuery({
    queryKey: ["channel-canvas", channelId],
    queryFn: () => {
      if (!channelId) {
        return Promise.reject(new Error("No channel selected"));
      }
      return getCanvas(channelId);
    },
    enabled: enabled && channelId !== null,
  });
}

export function useCanvasHistoryQuery(
  channelId: string | null,
  enabled: boolean,
) {
  return useInfiniteQuery<CanvasHistoryResponse>({
    queryKey: ["channel-canvas-history", channelId],
    queryFn: ({ pageParam }) => {
      if (!channelId) {
        return Promise.reject(new Error("No channel selected"));
      }
      return getCanvasHistory(channelId, {
        cursor: (pageParam as CanvasHistoryCursor | null) ?? null,
      });
    },
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    initialPageParam: null,
    enabled: enabled && channelId !== null,
  });
}

export function useSetCanvasMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: {
      content: string;
      expectedRevision?: string | null;
    }) => {
      if (!channelId) {
        return Promise.reject(new Error("No channel selected"));
      }
      return setCanvas({
        channelId,
        content: input.content,
        expectedRevision: input.expectedRevision ?? null,
      });
    },
    // Invalidate on every settled outcome, not just success: an accepted
    // write reported as CANVAS_SUPERSEDED is durable and in history, yet it
    // rejects the mutation — the UI tells the user to reload and restore the
    // retained revision, so the stale current/history caches must refetch on
    // that rejection too. The pre-publish conflict paths and plain network
    // failures also mean the canvas may have moved, so a refetch is correct
    // there as well.
    onSettled: () => {
      if (channelId) {
        void queryClient.invalidateQueries({
          queryKey: ["channel-canvas", channelId],
        });
        void queryClient.invalidateQueries({
          queryKey: ["channel-canvas-history", channelId],
        });
      }
    },
  });
}
