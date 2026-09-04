import { useQuery } from "@tanstack/react-query";

import type { RelayMeetingsCapability } from "@/features/meetings/api";
import { fetchMeetingsCapability } from "@/features/meetings/relay";
import { useCommunities } from "@/features/communities/useCommunities";

/**
 * The active community relay's advertised Meetings capability, or `null` when
 * the relay hasn't opted in (`BUZZ_HIVETALK_API_ROOT` unset) or is unreachable.
 *
 * Phase 3 uses `capability === null` to hide the Meetings tab and sidebar
 * entry entirely.
 *
 * `isUnavailable` is the narrower signal for hiding navigation: it is true only
 * once the probe has *settled* on "this relay has no Meetings" (`/info` answered
 * and did not advertise the capability). A thrown probe — relay unreachable,
 * query error — leaves it false so the nav entry stays put through a transient
 * blip instead of flickering out and back.
 */
export function useMeetingsCapability(): {
  capability: RelayMeetingsCapability | null;
  isLoading: boolean;
  isUnavailable: boolean;
} {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";

  const query = useQuery({
    enabled: relayUrl.length > 0,
    queryFn: ({ signal }) => fetchMeetingsCapability(relayUrl, signal),
    queryKey: ["relay-capability", relayUrl, "meetings"],
    retry: 1,
    staleTime: 5 * 60 * 1_000,
  });

  return {
    capability: query.data ?? null,
    isLoading: query.isLoading,
    isUnavailable: query.isSuccess && query.data === null,
  };
}
