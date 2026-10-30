import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { relayKlipyEndpoints } from "@/features/gifs/relay";

/** The active community relay's KLIPY endpoints, shared across GIF surfaces. */
export function useRelayKlipyCapability() {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const capabilityQuery = useQuery({
    enabled: relayUrl.length > 0,
    queryFn: ({ signal }) => relayKlipyEndpoints(relayUrl, signal),
    queryKey: ["relay-capability", relayUrl, "klipy"],
    retry: 1,
    staleTime: 5 * 60 * 1_000,
  });
  const searchPath = capabilityQuery.data?.searchPath ?? "";
  const sharePath = capabilityQuery.data?.sharePath ?? "";
  return {
    available: searchPath.length > 0 && sharePath.length > 0,
    loading: capabilityQuery.isLoading,
    relayUrl,
    searchPath,
    sharePath,
  };
}
