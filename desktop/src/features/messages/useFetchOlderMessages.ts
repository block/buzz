import { useCallback, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  channelMessagesKey,
  mergeTimelineHistoryMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import type { Channel, RelayEvent } from "@/shared/api/types";

const OLDER_MESSAGES_BATCH_SIZE = 100;

export function useFetchOlderMessages(channel: Channel | null) {
  const queryClient = useQueryClient();
  const channelId = channel?.id ?? null;
  const [isFetchingOlder, setIsFetchingOlder] = useState(false);
  const [hasOlderMessages, setHasOlderMessages] = useState(true);
  const isFetchingOlderRef = useRef(false);
  const hasOlderMessagesRef = useRef(true);

  const previousChannelIdRef = useRef(channelId);
  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    hasOlderMessagesRef.current = true;
    setHasOlderMessages(true);
  }

  const fetchOlder = useCallback(async () => {
    if (
      !channelId ||
      isFetchingOlderRef.current ||
      !hasOlderMessagesRef.current
    ) {
      return;
    }

    const queryKey = channelMessagesKey(channelId);
    const currentMessages =
      queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
    if (currentMessages.length === 0) {
      hasOlderMessagesRef.current = false;
      setHasOlderMessages(false);
      return;
    }

    // Use the oldest timestamp directly — `until` is inclusive so the relay will
    // return the boundary message again, but `sortMessages` deduplicates by id.
    // Subtracting 1 risks skipping messages that share the same second.
    const oldestTimestamp = currentMessages[0].created_at;
    isFetchingOlderRef.current = true;
    setIsFetchingOlder(true);

    try {
      const olderMessages = await relayClient.fetchChannelHistoryBefore(
        channelId,
        oldestTimestamp,
        OLDER_MESSAGES_BATCH_SIZE,
      );

      if (olderMessages.length < OLDER_MESSAGES_BATCH_SIZE) {
        hasOlderMessagesRef.current = false;
        setHasOlderMessages(false);
      }

      if (olderMessages.length > 0) {
        queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
          mergeTimelineHistoryMessages(current, olderMessages),
        );

        const updatedMessages =
          queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
        if (
          updatedMessages.length > 0 &&
          updatedMessages[0].created_at === oldestTimestamp
        ) {
          hasOlderMessagesRef.current = false;
          setHasOlderMessages(false);
        }
      }
    } catch (error) {
      console.error("Failed to fetch older messages", channelId, error);
    } finally {
      isFetchingOlderRef.current = false;
      setIsFetchingOlder(false);
    }
  }, [channelId, queryClient]);

  return { fetchOlder, isFetchingOlder, hasOlderMessages };
}
