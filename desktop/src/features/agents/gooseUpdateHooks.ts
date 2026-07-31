import { useQuery } from "@tanstack/react-query";

import { checkGooseUpdateStatus } from "@/shared/api/tauriGooseUpdates";

export const gooseUpdateStatusQueryKey = ["goose-update-status"] as const;

/** Settings-scoped, session-cached check for a newer stable Goose release. */
export function useGooseUpdateStatusQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    gcTime: Number.POSITIVE_INFINITY,
    queryFn: checkGooseUpdateStatus,
    queryKey: gooseUpdateStatusQueryKey,
    refetchOnMount: false,
    refetchOnReconnect: false,
    refetchOnWindowFocus: false,
    retry: false,
    retryOnMount: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
}
