import { useQuery } from "@tanstack/react-query";

import { checkGooseUpdateStatus } from "@/shared/api/tauriGooseUpdates";

export const gooseUpdateStatusQueryKey = ["goose-update-status"] as const;

const GOOSE_UPDATE_STATUS_CACHE_MS = 60 * 60 * 1_000;

/** Settings-scoped, session-cached check for a newer stable Goose release. */
export function useGooseUpdateStatusQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    gcTime: GOOSE_UPDATE_STATUS_CACHE_MS,
    queryFn: checkGooseUpdateStatus,
    queryKey: gooseUpdateStatusQueryKey,
    retry: false,
    staleTime: GOOSE_UPDATE_STATUS_CACHE_MS,
  });
}
