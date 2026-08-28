import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { MeetingError } from "@/features/meetings/api";
import {
  listRooms,
  listRoomsByPubkey,
  registerRoom,
} from "@/features/meetings/relay";
import { useMeetingsCapability } from "@/features/meetings/useMeetingsCapability";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

/** Active-room list poll cadence while the Meetings screen is focused. */
export const MEETING_ROOMS_REFETCH_INTERVAL_MS = 15_000;
/** Stale gate — the list has no relay push, so a focus return refreshes it. */
export const MEETING_ROOMS_STALE_TIME_MS = 10_000;

/** Query keys for the Meetings feature; a shared prefix so a mutation can
 * invalidate every room list for the active relay in one call. */
export const meetingsQueryKeys = {
  all: (relayUrl: string) => ["meetings", relayUrl] as const,
  rooms: (relayUrl: string) => ["meetings", relayUrl, "rooms"] as const,
  myRooms: (relayUrl: string, pubkey: string) =>
    ["meetings", relayUrl, "my-rooms", pubkey] as const,
};

function useActiveRelayUrl(): string {
  const { activeCommunity } = useCommunities();
  return activeCommunity?.relayUrl ?? "";
}

/** All active meeting rooms advertised by the community relay. */
export function useMeetingRoomsQuery() {
  const relayUrl = useActiveRelayUrl();
  const refetchInterval = useFocusedRefetchInterval(
    MEETING_ROOMS_REFETCH_INTERVAL_MS,
  );
  return useQuery({
    enabled: relayUrl.length > 0,
    queryFn: ({ signal }) => listRooms(relayUrl, signal),
    queryKey: meetingsQueryKeys.rooms(relayUrl),
    refetchInterval,
    refetchOnWindowFocus: true,
    staleTime: MEETING_ROOMS_STALE_TIME_MS,
  });
}

/** Active rooms hosted by the signed-in identity. Empty until it has one. */
export function useMyMeetingRoomsQuery() {
  const relayUrl = useActiveRelayUrl();
  const pubkey = useIdentityQuery().data?.pubkey ?? "";
  const refetchInterval = useFocusedRefetchInterval(
    MEETING_ROOMS_REFETCH_INTERVAL_MS,
  );
  return useQuery({
    enabled: relayUrl.length > 0 && pubkey.length > 0,
    queryFn: ({ signal }) => listRoomsByPubkey(relayUrl, pubkey, signal),
    queryKey: meetingsQueryKeys.myRooms(relayUrl, pubkey),
    refetchInterval,
    refetchOnWindowFocus: true,
    staleTime: MEETING_ROOMS_STALE_TIME_MS,
  });
}

/**
 * Register (create) a meeting room. Resolves with the `RegisteredRoom`; rejects
 * with a `MeetingError` — callers branch on `.kind` for the hosting/subscription
 * paths (Phase 5).
 */
export function useRegisterRoomMutation() {
  const relayUrl = useActiveRelayUrl();
  const { capability } = useMeetingsCapability();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (roomName: string) => {
      if (!capability) {
        throw new MeetingError(
          "not_configured",
          0,
          "Meetings isn't enabled on this relay.",
        );
      }
      return registerRoom(relayUrl, capability, roomName);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: meetingsQueryKeys.all(relayUrl),
      });
    },
  });
}
