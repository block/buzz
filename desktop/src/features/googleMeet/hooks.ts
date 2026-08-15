import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cancelGoogleMeetConnect,
  createInstantGoogleMeet,
  disconnectGoogleMeetAccount,
  getGoogleMeetConnectionStatus,
  startGoogleMeetConnect,
  type GoogleMeetInfo,
} from "@/shared/api/tauri";

const GOOGLE_MEET_CONNECTION_QUERY_KEY = ["google-meet-connection"] as const;

/** Whether this device has a saved Google account connection. Does not
 * verify the token is still valid — `createInstantGoogleMeet` does that and
 * clears the stored token itself if the refresh fails. */
export function useGoogleMeetConnectionQuery() {
  return useQuery({
    queryKey: GOOGLE_MEET_CONNECTION_QUERY_KEY,
    queryFn: getGoogleMeetConnectionStatus,
    // Cheap local keyring read, not a network call — fine to treat as fresh
    // for the lifetime of the settings/channel view rather than re-fetching
    // on every focus.
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useConnectGoogleMeetMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: startGoogleMeetConnect,
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: GOOGLE_MEET_CONNECTION_QUERY_KEY,
      });
    },
  });
}

export function useCancelGoogleMeetConnectMutation() {
  return useMutation({ mutationFn: cancelGoogleMeetConnect });
}

export function useDisconnectGoogleMeetMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: disconnectGoogleMeetAccount,
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: GOOGLE_MEET_CONNECTION_QUERY_KEY,
      });
    },
  });
}

export function useCreateInstantGoogleMeetMutation() {
  const queryClient = useQueryClient();
  return useMutation<GoogleMeetInfo, Error, void>({
    mutationFn: createInstantGoogleMeet,
    onError: (error) => {
      // A dead refresh token is deleted server-side on failure — reflect
      // that in the cached connection status immediately rather than
      // waiting for the next explicit status check.
      if (error.message.toLowerCase().includes("reconnect")) {
        void queryClient.invalidateQueries({
          queryKey: GOOGLE_MEET_CONNECTION_QUERY_KEY,
        });
      }
    },
  });
}
