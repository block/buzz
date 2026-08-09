import { useQuery } from "@tanstack/react-query";

import { fetchTeamCatalogPublications } from "@/features/agents/lib/teamCatalogRelay";
import { useCommunities } from "@/features/communities/useCommunities";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

export function teamCatalogQueryKey(relayUrl: string) {
  return ["shared-team-catalog", normalizeRelayUrl(relayUrl)] as const;
}

/** Read relay-confirmed shared team heads without exposing mutations. */
export function useTeamCatalogQuery(options?: { enabled?: boolean }) {
  const { activeCommunity } = useCommunities();
  const relayUrl = normalizeRelayUrl(activeCommunity?.relayUrl ?? "");

  return useQuery({
    queryKey: teamCatalogQueryKey(relayUrl),
    queryFn: fetchTeamCatalogPublications,
    staleTime: 30_000,
    refetchInterval: 5 * 60_000,
    refetchIntervalInBackground: false,
    enabled: (options?.enabled ?? true) && relayUrl.length > 0,
  });
}
