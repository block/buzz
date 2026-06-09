import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import type {
  RelayEvent,
  UserStatus,
  UserStatusLookup,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { KIND_USER_STATUS } from "@/shared/constants/kinds";

function normalizePubkeys(pubkeys: string[]) {
  return [...new Set(pubkeys.map((pk) => normalizePubkey(pk)))]
    .filter((pk) => pk.length > 0)
    .sort();
}

export function userStatusQueryKey(pubkeys: string[]) {
  return ["user-status", ...normalizePubkeys(pubkeys)] as const;
}

export function parseUserStatusEvent(event: RelayEvent): {
  pubkey: string;
  text: string;
  emoji: string;
  updatedAt: number;
} {
  const emojiTag = event.tags.find(
    (tag) => tag[0] === "emoji" && tag.length >= 2,
  );
  return {
    pubkey: normalizePubkey(event.pubkey),
    text: event.content,
    emoji: emojiTag?.[1] ?? "",
    updatedAt: event.created_at,
  };
}

export function useUserStatusQuery(pubkeys: string[]) {
  const normalizedPubkeys = normalizePubkeys(pubkeys);
  const enabled = normalizedPubkeys.length > 0;

  return useQuery<UserStatusLookup>({
    enabled,
    queryKey: userStatusQueryKey(normalizedPubkeys),
    queryFn: async () => {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_USER_STATUS],
        authors: normalizedPubkeys,
        "#d": ["general"],
        limit: normalizedPubkeys.length,
      });

      const lookup: UserStatusLookup = {};
      for (const pk of normalizedPubkeys) {
        lookup[pk] = null;
      }

      for (const event of events) {
        const parsed = parseUserStatusEvent(event);
        const existing = lookup[parsed.pubkey];
        if (!existing || parsed.updatedAt > existing.updatedAt) {
          lookup[parsed.pubkey] =
            parsed.text || parsed.emoji
              ? {
                  text: parsed.text,
                  emoji: parsed.emoji,
                  updatedAt: parsed.updatedAt,
                }
              : null;
        }
      }

      return lookup;
    },
    staleTime: 60_000,
    refetchInterval: 120_000,
  });
}

export function useUserStatusSubscription() {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    let unsub: (() => Promise<void>) | null = null;
    let isCancelled = false;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    function handleStatusEvent(event: RelayEvent) {
      if (isCancelled) return;
      const dTag = event.tags.find((t) => t[0] === "d");
      if (dTag?.[1] !== "general") return;
      const parsed = parseUserStatusEvent(event);
      const status: UserStatus | null =
        parsed.text || parsed.emoji
          ? {
              text: parsed.text,
              emoji: parsed.emoji,
              updatedAt: parsed.updatedAt,
            }
          : null;

      queryClient.setQueriesData<UserStatusLookup>(
        { queryKey: ["user-status"] },
        (old) => {
          if (!old || !(parsed.pubkey in old)) return old;
          const existing = old[parsed.pubkey];
          if (existing && existing.updatedAt >= parsed.updatedAt) return old;
          return { ...old, [parsed.pubkey]: status };
        },
      );
    }

    function subscribeWithRetry(attempt = 0) {
      if (isCancelled) return;
      void relayClient
        .subscribeToUserStatusUpdates(handleStatusEvent)
        .then((unsubFn) => {
          if (isCancelled) {
            void unsubFn();
            return;
          }
          unsub = unsubFn;
        })
        .catch(() => {
          if (!isCancelled) {
            const delay = Math.min(1000 * 2 ** attempt, 30_000);
            retryTimer = setTimeout(
              () => subscribeWithRetry(attempt + 1),
              delay,
            );
          }
        });
    }
    subscribeWithRetry();

    const unsubReconnect = relayClient.subscribeToReconnects(() => {
      if (!isCancelled)
        void queryClient.invalidateQueries({ queryKey: ["user-status"] });
    });

    return () => {
      isCancelled = true;
      unsubReconnect();
      if (retryTimer) clearTimeout(retryTimer);
      if (unsub) void unsub();
    };
  }, [queryClient]);
}

export function useSetUserStatusMutation(pubkey?: string) {
  const queryClient = useQueryClient();
  const normalizedPubkey = normalizePubkey(pubkey ?? "");

  return useMutation({
    mutationFn: async ({ text, emoji }: { text: string; emoji: string }) => {
      await relayClient.publishUserStatus(text, emoji);
      return { text, emoji };
    },
    onSuccess: ({ text, emoji }) => {
      if (normalizedPubkey.length === 0) return;

      const status: UserStatus | null =
        text || emoji
          ? { text, emoji, updatedAt: Math.floor(Date.now() / 1_000) }
          : null;

      queryClient.setQueryData<UserStatusLookup>(
        userStatusQueryKey([normalizedPubkey]),
        (old) => ({ ...(old ?? {}), [normalizedPubkey]: status }),
      );

      queryClient.setQueriesData<UserStatusLookup>(
        { queryKey: ["user-status"] },
        (old) => {
          if (!old || !(normalizedPubkey in old)) return old;
          return { ...old, [normalizedPubkey]: status };
        },
      );
    },
  });
}
