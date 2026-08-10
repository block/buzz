import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

export const relayAgentsQueryKey = ["relay-agents"] as const;

export function relayAgentsQueryKeyForScope(
  relayUrl: string,
  communityId = "",
  accountPubkey = "",
) {
  return [
    relayAgentsQueryKey[0],
    normalizeRelayUrl(relayUrl),
    communityId,
    accountPubkey,
  ] as const;
}

/**
 * Tenant scope for relay-directory queries. Cache keys include the active
 * canonical relay URL plus the community and account identity so a
 * relay/community switch can never briefly serve another tenant's records
 * out of the persisted query cache.
 */
export function useRelayAgentsQueryScope() {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const relayUrl = normalizeRelayUrl(activeCommunity?.relayUrl ?? "");
  const communityId = activeCommunity?.id ?? "";
  const accountPubkey = identityQuery.data?.pubkey?.toLowerCase() ?? "";

  return {
    accountPubkey,
    communityId,
    enabled: relayUrl.length > 0 && accountPubkey.length > 0,
    queryKey: relayAgentsQueryKeyForScope(relayUrl, communityId, accountPubkey),
  } as const;
}
