import { useCallback, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { pageOlderMessagesUntilRowFloor } from "@/features/messages/lib/pageOlderMessages";
import type { Channel, RelayEvent } from "@/shared/api/types";

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

    isFetchingOlderRef.current = true;
    setIsFetchingOlder(true);
    try {
      const { hasOlderMessages: more } = await pageOlderMessagesUntilRowFloor(
        queryClient,
        channelId,
        () => previousChannelIdRef.current === channelId,
      );
      if (!more) {
        hasOlderMessagesRef.current = false;
        setHasOlderMessages(false);
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
