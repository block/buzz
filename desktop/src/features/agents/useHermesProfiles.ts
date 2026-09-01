import { useQuery } from "@tanstack/react-query";

import { discoverHermesProfiles } from "@/shared/api/tauriHermesProfiles";

export const hermesProfilesQueryKey = ["hermes-profiles"] as const;

/**
 * Local Hermes profile inventory for runtime setup.
 *
 * Disabled by default at call sites until the Hermes harness is selected: the
 * command performs a bounded process probe and should not run for unrelated
 * agent runtimes.
 */
export function useHermesProfilesQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: hermesProfilesQueryKey,
    queryFn: discoverHermesProfiles,
    staleTime: 15_000,
  });
}
