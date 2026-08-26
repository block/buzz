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
    onSuccess: () => {
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
