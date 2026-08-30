import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { reduceWorkReports } from "@/features/messages/lib/workReport";
import { relayClient } from "@/shared/api/relayClient";
import { KIND_WORK_REPORT } from "@/shared/constants/kinds";

export function workReportQueryKey(channelId: string, rootId: string) {
  return ["work-report", channelId, rootId] as const;
}

export function useWorkReport(channelId: string | null, rootId: string | null) {
  const queryClient = useQueryClient();
  const key = React.useMemo(
    () => workReportQueryKey(channelId ?? "none", rootId ?? "none"),
    [channelId, rootId],
  );
  const enabled = channelId !== null && rootId !== null;
  const query = useQuery({
    queryKey: key,
    enabled,
    queryFn: async () => {
      if (!channelId || !rootId) return null;
      const events = await relayClient.fetchEvents({
        kinds: [KIND_WORK_REPORT],
        limit: 100,
        "#h": [channelId],
        "#e": [rootId],
      });
      return reduceWorkReports(events, channelId, rootId);
    },
    staleTime: 0,
  });

  React.useEffect(() => {
    if (!channelId || !rootId) return;
    let disposed = false;
    let unsubscribe: (() => Promise<void>) | null = null;
    void relayClient
      .subscribeLive(
        {
          kinds: [KIND_WORK_REPORT],
          limit: 0,
          "#h": [channelId],
          "#e": [rootId],
        },
        () => {
          void queryClient.invalidateQueries({ queryKey: key });
        },
      )
      .then((dispose) => {
        if (disposed) void dispose();
        else unsubscribe = dispose;
      })
      .catch((error) =>
        console.error("Failed to subscribe to work reports", error),
      );
    return () => {
      disposed = true;
      void unsubscribe?.();
    };
  }, [channelId, key, queryClient, rootId]);

  return query;
}
