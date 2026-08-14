import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MEMBER_ADDED_NOTIFICATION,
  KIND_MEMBER_REMOVED_NOTIFICATION,
} from "@/shared/constants/kinds";

const MEMBERSHIP_NOTIFICATION_RETRY_BASE_MS = 1_000;
const MEMBERSHIP_NOTIFICATION_RETRY_MAX_MS = 30_000;

export function useMembershipNotifications(currentPubkey?: string) {
  const queryClient = useQueryClient();
  const normalizedCurrentPubkey = currentPubkey?.trim().toLowerCase() ?? "";

  const handleMembershipNotification = React.useEffectEvent(
    (event: RelayEvent) => {
      const channelId = getChannelIdFromTags(event.tags);

      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      if (!channelId) {
        return;
      }

      void queryClient.invalidateQueries({
        queryKey: ["channels", channelId, "detail"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["channels", channelId, "members"],
      });
    },
  );

  React.useEffect(() => {
    if (normalizedCurrentPubkey.length === 0) {
      return;
    }

    let isCancelled = false;
    let retryTimeout: number | undefined;
    let retryAttempt = 0;
    let dispose: (() => Promise<void>) | undefined;

    const subscribe = async (): Promise<boolean> => {
      try {
        const nextDispose = await relayClient.subscribeLive(
          {
            kinds: [
              KIND_MEMBER_ADDED_NOTIFICATION,
              KIND_MEMBER_REMOVED_NOTIFICATION,
            ],
            "#p": [normalizedCurrentPubkey],
            limit: 50,
            since: Math.floor(Date.now() / 1_000) - 30,
          },
          (event) => {
            if (!isCancelled) {
              handleMembershipNotification(event);
            }
          },
        );
        if (isCancelled) {
          void nextDispose().catch(() => {});
          return true;
        }
        dispose = nextDispose;
        return true;
      } catch (error) {
        console.error("Failed to subscribe to membership notifications", error);
        return false;
      }
    };

    const run = async () => {
      const ok = await subscribe();
      if (isCancelled || ok) {
        return;
      }

      const delayMs = Math.min(
        MEMBERSHIP_NOTIFICATION_RETRY_BASE_MS * 2 ** retryAttempt,
        MEMBERSHIP_NOTIFICATION_RETRY_MAX_MS,
      );
      retryAttempt += 1;
      retryTimeout = window.setTimeout(() => {
        retryTimeout = undefined;
        void run();
      }, delayMs);
    };

    void run();

    return () => {
      isCancelled = true;
      if (retryTimeout !== undefined) {
        window.clearTimeout(retryTimeout);
      }
      if (dispose) {
        void dispose().catch(() => {});
      }
    };
  }, [normalizedCurrentPubkey]);
}
