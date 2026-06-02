import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  addCustomEmoji,
  listCustomEmoji,
  removeCustomEmoji,
} from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/**
 * React-query hooks for the relay-owned custom emoji set (NIP-30, kind:30030).
 *
 * The set is relay-global (one canonical "workspace" list), so the query key is
 * stable — not keyed by channel or pubkey. Mirrors `user-status/hooks.ts`.
 */

export const customEmojiQueryKey = ["custom-emoji"] as const;

export function useCustomEmojiQuery() {
  return useQuery<CustomEmoji[]>({
    queryKey: customEmojiQueryKey,
    queryFn: listCustomEmoji,
    // The set changes rarely; avoid refetch storms while the picker is open.
    staleTime: 60_000,
  });
}

/**
 * Convenience accessor returning the emoji list (empty array while loading).
 * Most consumers (renderer, picker, send path) just want the array.
 */
export function useCustomEmoji(): CustomEmoji[] {
  return useCustomEmojiQuery().data ?? [];
}

export function useAddCustomEmojiMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ shortcode, url }: { shortcode: string; url: string }) =>
      addCustomEmoji(shortcode, url),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
    },
  });
}

export function useRemoveCustomEmojiMutation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (shortcode: string) => removeCustomEmoji(shortcode),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: customEmojiQueryKey });
    },
  });
}
