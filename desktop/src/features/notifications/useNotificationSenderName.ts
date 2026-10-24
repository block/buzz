import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  usersBatchEntryKey,
  type UsersBatchEntry,
} from "@/features/profile/hooks";
import {
  readCachedUserLabels,
  writeCachedUserLabels,
} from "@/features/profile/lib/userLabelStorage";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import { senderNameFromSummary } from "./lib/senderName";

/**
 * Synchronous sender-name lookup for live desktop notifications (DMs and
 * thread replies). Toasts must never wait on the network, so resolution is
 * cache-only: the per-pubkey `users-batch-entry` React Query cache first
 * (kept warm by the sidebar and home feed), then the persisted label cache
 * (survives restart). A cold miss returns `null` — the caller falls back to
 * neutral copy — and kicks off a background fetch so the sender's next
 * message resolves.
 */
export function useNotificationSenderName(): (
  pubkey: string | undefined,
) => string | null {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";

  return React.useCallback(
    (pubkey) => {
      const normalized = pubkey?.trim().toLowerCase() ?? "";
      if (!normalized) {
        return null;
      }

      const entry = queryClient.getQueryData<UsersBatchEntry>(
        usersBatchEntryKey(normalized),
      );
      if (entry) {
        // A stale name still beats no name for a toast; relay-confirmed
        // misses (summary: null) correctly resolve to the fallback copy.
        return senderNameFromSummary(entry.summary);
      }

      const cached = relayUrl
        ? readCachedUserLabels(relayUrl, [normalized])
        : undefined;
      const cachedSummary = cached?.profiles[normalized];
      if (cachedSummary) {
        return senderNameFromSummary(cachedSummary);
      }

      void getUsersBatch([normalized])
        .then((fresh) => {
          queryClient.setQueryData<UsersBatchEntry>(
            usersBatchEntryKey(normalized),
            {
              summary: fresh.profiles[normalized] ?? null,
              fetchedAt: Date.now(),
            },
          );
          if (relayUrl) {
            writeCachedUserLabels(relayUrl, fresh.profiles, fresh.missing);
          }
        })
        .catch(() => {
          // Warm-up is best effort; the toast already shipped with fallback copy.
        });
      return null;
    },
    [queryClient, relayUrl],
  );
}
