import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { MeetingError } from "@/features/meetings/api";
import type { ModerationAction } from "@/features/meetings/relay";
import {
  getMeetingToken,
  getPaymentStatus,
  getPlans,
  getSubscription,
  listRooms,
  listRoomsByPubkey,
  moderateRoom,
  registerRoom,
  subscribe,
} from "@/features/meetings/relay";
import { shouldStopPollingPayment } from "@/features/meetings/ui/subscribeState";
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
  token: (relayUrl: string, room: string) =>
    ["meetings", relayUrl, "token", room] as const,
  plans: (relayUrl: string) => ["meetings", relayUrl, "plans"] as const,
  subscription: (relayUrl: string, pubkey: string) =>
    ["meetings", relayUrl, "subscription", pubkey] as const,
  payment: (relayUrl: string, intentId: string) =>
    ["meetings", relayUrl, "payment", intentId] as const,
};

/** Poll cadence for a pending payment intent (plan §5.1: ~3s). */
export const PAYMENT_STATUS_POLL_INTERVAL_MS = 3_000;

/** Stop polling `payment/status` after this many consecutive *server/network*
 * failures so a dead endpoint doesn't loop forever (`retry: false`). A 429 does
 * not count toward this — it's an expected backpressure signal, not a dead
 * endpoint. */
export const PAYMENT_STATUS_MAX_POLL_FAILURES = 5;
/** Fallback backoff for a `payment/status` 429 with no parseable `retry in Ns`. */
export const PAYMENT_STATUS_RATE_LIMIT_BACKOFF_MS = 10_000;
/** Plans rarely change within a session. */
export const MEETING_PLANS_STALE_TIME_MS = 5 * 60 * 1_000;

/** LiveKit access tokens are time-limited; refetch a still-mounted call view's
 * token no more than once a minute. */
export const MEETING_TOKEN_STALE_TIME_MS = 60_000;

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

/**
 * Fetch the LiveKit access token + SFU URL for `room`. Rejects with a
 * `MeetingError` — a 402 routes callers to the Phase 5 hosting path, same as
 * `useRegisterRoomMutation`. Disabled until the relay capability and a signed-in
 * identity are both known.
 */
export function useMeetingTokenQuery(room: string) {
  const relayUrl = useActiveRelayUrl();
  const { capability } = useMeetingsCapability();
  const identity = useIdentityQuery().data;
  const pubkey = identity?.pubkey ?? "";
  const participantName = identity?.displayName?.trim() || "Guest";

  return useQuery({
    enabled:
      relayUrl.length > 0 &&
      capability !== null &&
      pubkey.length > 0 &&
      room.length > 0,
    queryFn: ({ signal }) => {
      if (!capability) {
        throw new MeetingError(
          "not_configured",
          0,
          "Meetings isn't enabled on this relay.",
        );
      }
      return getMeetingToken(
        relayUrl,
        capability,
        room,
        participantName,
        pubkey,
        signal,
      );
    },
    queryKey: meetingsQueryKeys.token(relayUrl, room),
    refetchOnWindowFocus: false,
    retry: false,
    staleTime: MEETING_TOKEN_STALE_TIME_MS,
  });
}

/** Subscription plans offered by the relay's HiveTalk provider. Membership-gated
 * but needs no HiveTalk signature, so it runs as soon as the relay is known. */
export function usePlansQuery() {
  const relayUrl = useActiveRelayUrl();
  return useQuery({
    enabled: relayUrl.length > 0,
    queryFn: ({ signal }) => getPlans(relayUrl, signal),
    queryKey: meetingsQueryKeys.plans(relayUrl),
    staleTime: MEETING_PLANS_STALE_TIME_MS,
  });
}

/**
 * The signed-in identity's HiveTalk subscription status for this relay. Drives
 * the Meetings-header badge; disabled until the relay capability and a signed-in
 * identity are both known. Rejects with a `MeetingError`.
 */
export function useSubscriptionQuery() {
  const relayUrl = useActiveRelayUrl();
  const { capability } = useMeetingsCapability();
  const pubkey = useIdentityQuery().data?.pubkey ?? "";
  return useQuery({
    enabled: relayUrl.length > 0 && capability !== null && pubkey.length > 0,
    queryFn: ({ signal }) => {
      if (!capability) {
        throw new MeetingError(
          "not_configured",
          0,
          "Meetings isn't enabled on this relay.",
        );
      }
      return getSubscription(relayUrl, capability, signal);
    },
    queryKey: meetingsQueryKeys.subscription(relayUrl, pubkey),
    refetchOnWindowFocus: false,
    retry: false,
    staleTime: MEETING_ROOMS_STALE_TIME_MS,
  });
}

/**
 * Start a subscription purchase. Resolves with the BOLT11 `SubscribeIntent`;
 * rejects with a `MeetingError` — a `409 pending_invoice` carries the existing
 * intent in `error.pendingInvoice`, which `SubscribeView` surfaces as-is.
 */
export function useSubscribeMutation() {
  const relayUrl = useActiveRelayUrl();
  const { capability } = useMeetingsCapability();
  return useMutation({
    mutationFn: (plan: string) => {
      if (!capability) {
        throw new MeetingError(
          "not_configured",
          0,
          "Meetings isn't enabled on this relay.",
        );
      }
      return subscribe(relayUrl, capability, plan);
    },
  });
}

/**
 * Poll a payment intent until it settles or expires. `enabled` should track the
 * invoice panel being mounted; the interval stops itself once the fetched
 * status is terminal so a settled intent isn't re-fetched forever.
 */
export function usePaymentStatusQuery(
  intentId: string | undefined,
  enabled: boolean,
) {
  const relayUrl = useActiveRelayUrl();
  const { capability } = useMeetingsCapability();
  return useQuery({
    enabled:
      enabled &&
      relayUrl.length > 0 &&
      capability !== null &&
      (intentId?.length ?? 0) > 0,
    queryFn: ({ signal }) => {
      if (!capability || !intentId) {
        throw new MeetingError(
          "not_configured",
          0,
          "Meetings isn't enabled on this relay.",
        );
      }
      return getPaymentStatus(relayUrl, capability, intentId, signal);
    },
    queryKey: meetingsQueryKeys.payment(relayUrl, intentId ?? ""),
    refetchInterval: (query) => {
      if (query.state.status === "error") {
        const error = query.state.error;
        // A 429 is expected backpressure mid-invoice — never give up on it,
        // just slow down. Honor the relay's `retry in Ns` when it sends one.
        if (error instanceof MeetingError && error.status === 429) {
          const secs = error.retryAfterSecs;
          return secs && secs > 0
            ? secs * 1_000
            : PAYMENT_STATUS_RATE_LIMIT_BACKOFF_MS;
        }
        // Give up on an endpoint that keeps failing with a real error —
        // `retry: false` means a persistent 5xx would otherwise loop every 3s
        // forever. `SubscribeView` surfaces the stalled state to the user.
        if (query.state.fetchFailureCount >= PAYMENT_STATUS_MAX_POLL_FAILURES) {
          return false;
        }
        return PAYMENT_STATUS_POLL_INTERVAL_MS;
      }
      const payment = query.state.data;
      if (!payment) return PAYMENT_STATUS_POLL_INTERVAL_MS;
      return shouldStopPollingPayment(payment, Date.now())
        ? false
        : PAYMENT_STATUS_POLL_INTERVAL_MS;
    },
    // Error state stops the interval; a window-focus return retries once so a
    // user who paid while the poll was dead isn't stranded forever.
    refetchOnWindowFocus: true,
    retry: false,
    staleTime: 0,
  });
}

/**
 * Host-control mutation. `livekitJwt` is the token returned by
 * `useMeetingTokenQuery`; HiveTalk authorizes each call from the `owner` /
 * `moderator` claims inside it. A successful call invalidates the room lists so
 * a kick/mute is reflected in participant counts on the next poll.
 */
export function useModerateRoomMutation(livekitJwt: string | undefined) {
  const relayUrl = useActiveRelayUrl();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: {
      action: ModerationAction;
      payload: Record<string, unknown>;
    }) => {
      if (!livekitJwt) {
        throw new MeetingError(
          "bad_signature",
          0,
          "You need to be connected to the room to use host controls.",
        );
      }
      return moderateRoom(relayUrl, input.action, livekitJwt, input.payload);
    },
    onSuccess: () => {
      // Room lists only. Invalidating the `all` prefix also matches the active
      // LiveKit token query, which would remint the JWT and bounce the host's
      // own call on every moderation action.
      void queryClient.invalidateQueries({
        queryKey: meetingsQueryKeys.rooms(relayUrl),
      });
      void queryClient.invalidateQueries({
        queryKey: ["meetings", relayUrl, "my-rooms"],
      });
    },
  });
}
