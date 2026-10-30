import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  fetchCommunityThreadPolicy,
  type ThreadPolicy,
} from "@/shared/api/communityProfile";

/** React Query key for one community's NIP-TP thread policy. */
export const communityThreadPolicyQueryKey = (relayUrl: string) =>
  ["communityThreadPolicy", relayUrl] as const;

const THREAD_POLICY_STALE_MS = 5 * 60_000;

/** Query for the active community's thread policy, read from NIP-11. */
export function useCommunityThreadPolicyQuery() {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl;
  return useQuery({
    queryKey: communityThreadPolicyQueryKey(relayUrl ?? ""),
    queryFn: () => fetchCommunityThreadPolicy(relayUrl ?? ""),
    enabled: relayUrl !== undefined,
    staleTime: THREAD_POLICY_STALE_MS,
  });
}

/**
 * The active community's thread policy. Reads `nested` (the NIP-TP default)
 * until the relay answers, and while it cannot be reached.
 */
export function useCommunityThreadPolicy(): ThreadPolicy {
  return useCommunityThreadPolicyQuery().data ?? "nested";
}
