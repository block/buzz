import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { relayKlipyEndpoints } from "@/features/gifs/relay";

/** The active relay's GIF endpoints; `available` is false without both. */
export function useKlipyCapability(enabled = true) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const query = useQuery({
    enabled: enabled && relayUrl.length > 0,
    queryFn: ({ signal }) => relayKlipyEndpoints(relayUrl, signal),
    queryKey: ["relay-capability", relayUrl, "klipy"],
    retry: 1,
    staleTime: 5 * 60 * 1_000,
  });
  const searchPath = query.data?.searchPath ?? "";
  const sharePath = query.data?.sharePath ?? "";
  return {
    available: searchPath.length > 0 && sharePath.length > 0,
    isPending: query.isPending && query.fetchStatus !== "idle",
    relayUrl,
    searchPath,
    sharePath,
  };
}
