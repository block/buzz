import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";

/**
 * Shared React Query key for the ACP runtime catalog. Every consumer (cheap or
 * forced) reads and writes this one entry, so a forced refresh updates the same
 * cache the hot-path `useAcpRuntimesQuery` renders from.
 */
export const acpRuntimesQueryKey = ["acp-runtimes"] as const;

/**
 * Run a forced (full re-discovery) refresh and write the result into the shared
 * runtime-catalog cache.
 *
 * This is the only path that pays the expensive discovery pipeline (cache
 * clear, PATH re-fetch, CLI auth probes). Surfaces that need fresh state call
 * it deliberately: Settings/onboarding on open and on their refresh buttons,
 * and the connect/install/save/delete mutations in `onSettled`. A bare
 * `invalidateQueries` would only re-run the cheap query path and never
 * re-probe, so the freshly-changed auth/catalog state would not be reflected.
 *
 * `fetchQuery` dedups concurrent callers for this key; the backend coalesces
 * forced runs as a second layer.
 */
export function refreshAcpRuntimes(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  return queryClient.fetchQuery({
    queryKey: acpRuntimesQueryKey,
    queryFn: () => discoverAcpRuntimes({ force: true }),
    staleTime: 0,
  });
}

/**
 * ACP runtimes query for surfaces that need fresh auth/version state: Settings
 * harness panels and onboarding. It reads from the shared cache but suppresses
 * the cheap mount-fetch (`refetchOnMount: false`) so the forced re-discovery it
 * fires on mount is the *only* fetch for this key — otherwise React Query could
 * dedup the forced probe onto an in-flight cheap fetch and the surface would
 * show stale auth. The observer still reflects the forced fetch's loading state
 * (shared query state), and `forceRefresh` drives explicit refresh buttons and
 * sign-in polling.
 */
export function useAcpRuntimesQueryForced(options?: { enabled?: boolean }) {
  const enabled = options?.enabled ?? true;
  const queryClient = useQueryClient();
  const query = useQuery({
    enabled,
    queryKey: acpRuntimesQueryKey,
    queryFn: () => discoverAcpRuntimes(),
    staleTime: 30 * 60_000,
    // The mount forceRefresh below is the fetcher; don't also fire a cheap one.
    refetchOnMount: false,
  });
  const forceRefresh = React.useCallback(
    () => refreshAcpRuntimes(queryClient),
    [queryClient],
  );
  React.useEffect(() => {
    if (enabled) void forceRefresh();
  }, [enabled, forceRefresh]);
  return { ...query, forceRefresh };
}
