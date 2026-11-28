import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  type AgentUsageSeries,
  getAgentUsageSeries,
  onAgentMetricsChanged,
} from "@/shared/api/tauriArchive";
import {
  buildLocalDayBoundaries,
  type LandscapeModel,
  MODELS_DEV_URL,
  parseModelsDevCatalog,
  type UsageWindowDays,
} from "./lib/modelDashboard";

const modelUsageQueryKeyRoot = ["model-dashboard", "usage"] as const;

/**
 * Locally archived NIP-AM usage for every agent over the last `days` local
 * days. Invalidates when new kind-44200 metrics are archived instead of
 * polling; boundaries are rebuilt on each mount, so a window left open past
 * midnight catches up on the next metric or remount.
 */
export function useModelUsageSeries(days: UsageWindowDays) {
  const queryClient = useQueryClient();
  const boundaries = React.useMemo(() => buildLocalDayBoundaries(days), [days]);

  React.useEffect(
    () =>
      onAgentMetricsChanged(() => {
        void queryClient.invalidateQueries({
          queryKey: modelUsageQueryKeyRoot,
        });
      }),
    [queryClient],
  );

  return useQuery<AgentUsageSeries>({
    queryKey: [...modelUsageQueryKeyRoot, boundaries.join(",")],
    queryFn: () => getAgentUsageSeries({ bucketBoundaries: boundaries }),
    staleTime: 60_000,
    gcTime: 5 * 60_000,
  });
}

/**
 * Public model catalog from models.dev (lab list prices, context, weights).
 * Fetched only while the Landscape tab is open, then cached for the session.
 */
export function useModelLandscape(enabled: boolean) {
  return useQuery<LandscapeModel[]>({
    queryKey: ["model-dashboard", "landscape"],
    queryFn: async ({ signal }) => {
      const response = await fetch(MODELS_DEV_URL, { signal });
      if (!response.ok) {
        throw new Error(`models.dev returned ${response.status}`);
      }
      return parseModelsDevCatalog(await response.json());
    },
    enabled,
    staleTime: 6 * 60 * 60_000,
    gcTime: 24 * 60 * 60_000,
    retry: 1,
  });
}
