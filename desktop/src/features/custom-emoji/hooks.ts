import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  KIND_EMOJI_SET,
  CUSTOM_EMOJI_SET_D_TAG,
  fetchOwnEmoji,
  listCustomEmoji,
  removeCustomEmoji,
  setCustomEmoji,
} from "@/shared/api/customEmoji";
import { relayClient } from "@/shared/api/relayClient";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/**
 * React-query hooks for the community custom emoji palette (NIP-30, kind:30030).
 *
 * The palette is the client-side UNION of every member's own kind:30030 set, so
 * the query key is stable — not keyed by channel or pubkey. Freshness comes from
 * three layers: a catch-up fetch (the query itself), a live subscription that
 * invalidates on any member's new 30030, and a 2-minute poll backstop in case a
 * live event is missed. Mirrors `user-status/hooks.ts`.
 */

export const customEmojiQueryKey = ["custom-emoji"] as const;

/** Query key for the caller's OWN editable 30030 set (distinct from the union). */
export const ownCustomEmojiQueryKey = ["custom-emoji-own"] as const;

export function useCustomEmojiQuery() {
  return useQuery<CustomEmoji[]>({
    queryKey: customEmojiQueryKey,
    queryFn: listCustomEmoji,
    // The palette changes rarely; avoid refetch storms while the picker is open,
    // but poll every 2 minutes as a backstop for any missed live event.
    staleTime: 60_000,
    refetchInterval: 120_000,
  });
}

/**
 * The caller's OWN custom emoji set — the only thing the settings card may add
 * to or remove from. Distinct from the community union (`useCustomEmojiQuery`),
 * which is read-only across members.
 */
export function useOwnCustomEmojiQuery() {
  return useQuery<CustomEmoji[]>({
    queryKey: ownCustomEmojiQueryKey,
    queryFn: fetchOwnEmoji,
    staleTime: 60_000,
  });
}

/**
 * Subscribe to every member's kind:30030 emoji sets and invalidate the palette
 * query whenever one arrives. Call once near the app root (alongside other
 * global live subscriptions). Safe to mount once; disposes on unmount.
 */
export function useCommunityEmojiLiveUpdates(): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    let disposed = false;
    let dispose: (() => void) | undefined;

    void relayClient
      .subscribeLive(
        { kinds: [KIND_EMOJI_SET], "#d": [CUSTOM_EMOJI_SET_D_TAG], limit: 0 },
        () => {
          void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
        },
      )
      .then((unsubscribe) => {
        if (disposed) {
          void unsubscribe();
        } else {
          dispose = () => {
            void unsubscribe();
          };
        }
      })
      .catch((error) => {
        console.error("Failed to subscribe to community custom emoji", error);
      });

    // Re-sync on reconnect: a member's 30030 published while we were
    // disconnected won't replay through the live sub, so invalidate to
    // trigger a fresh catch-up fetch (don't wait for the 2-min poll).
    const unsubReconnect = relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
    });

    return () => {
      disposed = true;
      unsubReconnect();
      dispose?.();
    };
  }, [queryClient]);
}

/**
 * Convenience accessor returning the emoji list (empty array while loading).
 * Most consumers (renderer, picker, send path) just want the array.
 */
export function useCustomEmoji(): CustomEmoji[] {
  return useCustomEmojiQuery().data ?? [];
}

export function useSetCustomEmojiMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ shortcode, url }: { shortcode: string; url: string }) =>
      setCustomEmoji(shortcode, url),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
      void queryClient.invalidateQueries({ queryKey: ownCustomEmojiQueryKey });
    },
  });
}

export function useRemoveCustomEmojiMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (shortcode: string) => removeCustomEmoji(shortcode),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
      void queryClient.invalidateQueries({ queryKey: ownCustomEmojiQueryKey });
    },
  });
}
