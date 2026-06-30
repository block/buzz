import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  connectSpotify,
  disconnectSpotify,
  getSpotifyDevices,
  getSpotifyPlaybackState,
  getSpotifyStatus,
  pauseSpotifyPlayback,
  seekSpotifyPlayback,
  skipSpotifyNext,
  skipSpotifyPrevious,
  startSpotifyPlayback,
  type SpotifyDeviceInput,
  type SpotifyPlaybackInput,
  type SpotifySeekInput,
} from "@/features/spotify/api";

export const spotifyStatusQueryKey = ["spotify-status"] as const;
export const spotifyDevicesQueryKey = ["spotify-devices"] as const;
export const spotifyPlaybackStateQueryKey = ["spotify-playback-state"] as const;

export function useSpotifyStatusQuery() {
  return useQuery({
    queryKey: spotifyStatusQueryKey,
    queryFn: getSpotifyStatus,
    staleTime: 60_000,
  });
}

export function useSpotifyDevicesQuery({ enabled }: { enabled: boolean }) {
  return useQuery({
    enabled,
    queryKey: spotifyDevicesQueryKey,
    queryFn: getSpotifyDevices,
    refetchInterval: enabled ? 30_000 : false,
    staleTime: 20_000,
  });
}

export function useSpotifyPlaybackStateQuery({
  enabled,
}: {
  enabled: boolean;
}) {
  return useQuery({
    enabled,
    queryKey: spotifyPlaybackStateQueryKey,
    queryFn: getSpotifyPlaybackState,
    refetchInterval: enabled ? 10_000 : false,
    staleTime: 5_000,
  });
}

export function useSpotifyConnectionMutations() {
  const queryClient = useQueryClient();
  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: spotifyStatusQueryKey });
    void queryClient.invalidateQueries({ queryKey: spotifyDevicesQueryKey });
    void queryClient.invalidateQueries({
      queryKey: spotifyPlaybackStateQueryKey,
    });
  };

  const connect = useMutation({
    mutationFn: connectSpotify,
    onSuccess: invalidate,
  });
  const disconnect = useMutation({
    mutationFn: disconnectSpotify,
    onSuccess: invalidate,
  });

  return { connect, disconnect };
}

export function useSpotifyPlaybackMutation() {
  const queryClient = useQueryClient();

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: spotifyDevicesQueryKey });
    void queryClient.invalidateQueries({
      queryKey: spotifyPlaybackStateQueryKey,
    });
  };

  return useMutation({
    mutationFn: (input?: SpotifyPlaybackInput) =>
      startSpotifyPlayback(input ?? {}),
    onSuccess: invalidate,
  });
}

export function useSpotifyPlaybackControlMutations() {
  const queryClient = useQueryClient();
  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: spotifyDevicesQueryKey });
    void queryClient.invalidateQueries({
      queryKey: spotifyPlaybackStateQueryKey,
    });
  };

  const pause = useMutation({
    mutationFn: (input?: SpotifyDeviceInput) =>
      pauseSpotifyPlayback(input ?? {}),
    onSuccess: invalidate,
  });
  const next = useMutation({
    mutationFn: (input?: SpotifyDeviceInput) => skipSpotifyNext(input ?? {}),
    onSuccess: invalidate,
  });
  const previous = useMutation({
    mutationFn: (input?: SpotifyDeviceInput) =>
      skipSpotifyPrevious(input ?? {}),
    onSuccess: invalidate,
  });
  const seek = useMutation({
    mutationFn: (input: SpotifySeekInput) => seekSpotifyPlayback(input),
    onSuccess: invalidate,
  });

  return { pause, next, previous, seek };
}
