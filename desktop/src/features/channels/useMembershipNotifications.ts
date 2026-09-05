import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { refreshChannelsWhenIdle } from "@/features/channels/refreshChannelsWhenIdle";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MEMBER_ADDED_NOTIFICATION,
  KIND_MEMBER_REMOVED_NOTIFICATION,
} from "@/shared/constants/kinds";
import {
  createTrailingDebounce,
  type TrailingDebounce,
} from "@/shared/lib/trailingDebounce";

const MEMBERSHIP_NOTIFICATION_RETRY_BASE_MS = 1_000;
const MEMBERSHIP_NOTIFICATION_RETRY_MAX_MS = 30_000;
const CHANNELS_INVALIDATE_DEBOUNCE_MS = 500;

export function useMembershipNotifications(currentPubkey?: string) {
  const queryClient = useQueryClient();
  const normalizedCurrentPubkey = currentPubkey?.trim().toLowerCase() ?? "";
  const channelsInvalidateRef = React.useRef<TrailingDebounce | null>(null);
  if (channelsInvalidateRef.current === null) {
    channelsInvalidateRef.current = createTrailingDebounce(() => {
      refreshChannelsWhenIdle({
        // Scope the gate to the list query itself. A prefix match would also
        // count ["channels", id, "detail"] and ["channels", id, "members"] —
        // the very fetches this handler kicks off — so the list refresh would
        // be held closed by its own siblings and re-armed indefinitely.
        isFetching: () =>
          queryClient.isFetching({ queryKey: channelsQueryKey, exact: true }),
        invalidate: () => {
          void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
        },
        reArm: () => channelsInvalidateRef.current?.trigger(),
      });
    }, CHANNELS_INVALIDATE_DEBOUNCE_MS);
  }

  const handleMembershipNotification = React.useEffectEvent(
    (event: RelayEvent) => {
      const channelId = getChannelIdFromTags(event.tags);

      channelsInvalidateRef.current?.trigger();
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
      channelsInvalidateRef.current?.cancel();
      if (dispose) {
        void dispose().catch(() => {});
      }
    };
  }, [normalizedCurrentPubkey]);
}
