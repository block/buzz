import { useCallback, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { countTopLevelTimelineRows } from "@/features/messages/lib/formatTimelineMessages";
import {
  channelMessagesKey,
  mergeTimelineHistoryMessages,
} from "@/features/messages/lib/messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import type { Channel, RelayEvent } from "@/shared/api/types";

const OLDER_MESSAGES_BATCH_SIZE = 100;

// One scroll-up should advance the timeline by a predictable, *visible* amount.
// Because thread replies collapse into their parent and non-content events
// never render, a single 100-message batch can add far fewer rows than that —
// so we page in additional batches until at least this many top-level rows have
// been added (or history runs out). Counting rows, not messages, keeps a
// reply-heavy window from feeling like the fetch did nothing.
const MIN_TOP_LEVEL_ROWS_PER_FETCH = 10;

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

    // Page in batches until the timeline has gained at least
    // MIN_TOP_LEVEL_ROWS_PER_FETCH *visible* rows or history is exhausted.
    // A single batch is fetched first; only reply-heavy windows that fall short
    // of the floor loop again. Each batch re-reads the oldest timestamp from the
    // cache so successive `until` values keep walking backward without gaps.
    const baselineRowCount = countTopLevelTimelineRows(currentMessages);
    try {
      while (hasOlderMessagesRef.current) {
        const messagesBeforeFetch =
          queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
        if (messagesBeforeFetch.length === 0) {
          break;
        }

        // Use the oldest timestamp directly — `until` is inclusive so the relay
        // will return the boundary message again, but `sortMessages`
        // deduplicates by id. Subtracting 1 risks skipping messages that share
        // the same second.
        const oldestTimestamp = messagesBeforeFetch[0].created_at;
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

        const rowsGained =
          countTopLevelTimelineRows(
            queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [],
          ) - baselineRowCount;
        if (rowsGained >= MIN_TOP_LEVEL_ROWS_PER_FETCH) {
          break;
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
