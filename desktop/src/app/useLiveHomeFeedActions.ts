import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { remindersQueryKey } from "@/features/reminders/hooks";
import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_APPROVAL_REQUEST,
  KIND_EVENT_REMINDER,
  KIND_REMINDER,
} from "@/shared/constants/kinds";

const HOME_FEED_ACTION_KINDS = [KIND_APPROVAL_REQUEST, KIND_REMINDER] as const;

export function useLiveHomeFeedActions(
  pubkey: string | undefined,
  onHomeFeedEvent: () => void,
) {
  const queryClient = useQueryClient();
  const handleLiveHomeFeedEvent = React.useEffectEvent(() => {
    onHomeFeedEvent();
  });
  const handleLiveReminderEvent = React.useEffectEvent(
    (normalizedPubkey: string) => {
      onHomeFeedEvent();
      void queryClient.invalidateQueries({
        queryKey: remindersQueryKey(normalizedPubkey),
      });
    },
  );

  React.useEffect(() => {
    const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
    if (!normalizedPubkey) {
      return;
    }

    let isCancelled = false;
    let disposers: Array<() => Promise<void>> = [];
    const since = Math.floor(Date.now() / 1_000);

    void Promise.allSettled([
      relayClient.subscribeLive(
        {
          kinds: [...HOME_FEED_ACTION_KINDS],
          "#p": [normalizedPubkey],
          limit: 50,
          since,
        },
        handleLiveHomeFeedEvent,
      ),
      relayClient.subscribeLive(
        {
          authors: [normalizedPubkey],
          kinds: [KIND_EVENT_REMINDER],
          limit: 50,
          since,
        },
        () => {
          handleLiveReminderEvent(normalizedPubkey);
        },
      ),
    ]).then((results) => {
      const nextDisposers = results.flatMap((result) =>
        result.status === "fulfilled" ? [result.value] : [],
      );
      for (const result of results) {
        if (result.status === "rejected") {
          console.error(
            "Failed to subscribe to live home feed actions",
            result.reason,
          );
        }
      }

      if (nextDisposers.length === 0) {
        return;
      }

      if (isCancelled) {
        void Promise.allSettled(nextDisposers.map((dispose) => dispose()));
      } else {
        disposers = nextDisposers;
      }
    });

    return () => {
      isCancelled = true;
      const currentDisposers = disposers;
      disposers = [];
      void Promise.allSettled(currentDisposers.map((dispose) => dispose()));
    };
  }, [pubkey]);
}
