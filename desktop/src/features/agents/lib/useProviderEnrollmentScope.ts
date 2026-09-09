import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";

/** Resolve the currently displayed community scope at the action boundary. */
export function useProviderEnrollmentScope() {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();

  return {
    expectedRelayUrl: activeCommunity?.relayUrl.trim() || undefined,
    expectedSignerPubkey:
      identityQuery.data?.pubkey.trim().toLowerCase() || undefined,
  };
}
