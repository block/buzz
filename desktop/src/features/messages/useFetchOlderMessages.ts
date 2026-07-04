import { useCallback, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { channelWindowKey } from "@/features/messages/lib/messageQueryKeys";
import {
  channelWindowHasMore,
  emptyChannelWindowStore,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { pageOlderMessagesUntilRowFloor } from "@/features/messages/lib/pageOlderMessages";
import type { Channel } from "@/shared/api/types";

export function useFetchOlderMessages(channel: Channel | null) {
  const queryClient = useQueryClient();
  const channelId = channel?.id ?? null;
  const [isFetchingOlder, setIsFetchingOlder] = useState(false);
  const isFetchingOlderRef = useRef(false);

  // Whether older history remains, derived reactively from the authoritative
  // window store rather than a private latch. A latch only reset on channelId
  // change went stale on reconnect: `refreshNewestWindow` replaces the newest
  // window with fresh `hasMore:true` rows, but the latch — flipped false when
  // the pre-reconnect window exhausted — kept the scroll observer uninstalled,
  // freezing paging at page one. Reading the store's tail `hasMore` self-heals:
  // the observer re-arms the moment the refreshed window reports more history.
  const windowKey = channelWindowKey(channelId ?? "none");
  const { data: hasOlderMessages = false } = useQuery({
    enabled: channelId !== null,
    queryKey: windowKey,
    select: channelWindowHasMore,
    // Passive subscription: the window store is written by the messages query
    // and the live subscription via setQueryData; this observer only reads.
    queryFn: () =>
      queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
      emptyChannelWindowStore(),
  });

  const fetchOlder = useCallback(async () => {
    if (!channelId || isFetchingOlderRef.current) {
      return;
    }
    const store =
      queryClient.getQueryData<ChannelWindowStore>(
        channelWindowKey(channelId),
      ) ?? emptyChannelWindowStore();
    if (!channelWindowHasMore(store)) {
      return;
    }

    isFetchingOlderRef.current = true;
    setIsFetchingOlder(true);
    try {
      await pageOlderMessagesUntilRowFloor(
        queryClient,
        channelId,
        () => channelId === channel?.id,
      );
    } catch (error) {
      console.error("Failed to fetch older messages", channelId, error);
    } finally {
      isFetchingOlderRef.current = false;
      setIsFetchingOlder(false);
    }
  }, [channel?.id, channelId, queryClient]);

  return { fetchOlder, isFetchingOlder, hasOlderMessages };
}
