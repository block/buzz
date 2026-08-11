import { useQuery } from "@tanstack/react-query";

import { getHomeFeed } from "@/shared/api/tauri";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

/** Keeps focused polling at the established 30-second cadence. */
export const HOME_FEED_REFETCH_INTERVAL_MS = 30_000;
/** Suppresses the expensive focus refetch until the home feed is old. */
export const HOME_FEED_FOCUS_STALE_TIME_MS = 5 * 60_000;

export function useHomeFeedQuery() {
  const connectionState = useRelayConnection();
  const connected = connectionState === "connected";
  const refetchInterval = useFocusedRefetchInterval(
    connected ? HOME_FEED_REFETCH_INTERVAL_MS : false,
  );

  return useQuery({
    queryKey: ["home-feed"],
    queryFn: () =>
      getHomeFeed({
        limit: 50,
        types: "mentions,needs_action,activity,agent_activity",
      }),
    staleTime: HOME_FEED_FOCUS_STALE_TIME_MS,
    gcTime: 5 * 60 * 1_000,
    // Pause background polling on degraded/stalled/disconnected connections.
    // The relay can't serve the request anyway, and the spurious failures
    // consume quota that the recovery path needs.
    refetchInterval,
    refetchOnWindowFocus: true,
  });
}
