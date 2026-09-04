import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

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
  const scopeRef = useRef({ channelId, active: true, fetching: false });
  if (scopeRef.current.channelId !== channelId) {
    scopeRef.current.active = false;
    scopeRef.current = { channelId, active: true, fetching: false };
  }
  const scope = scopeRef.current;
  const [fetchingScope, setFetchingScope] = useState<typeof scope | null>(null);
  useEffect(() => {
    scope.active = true;
    return () => {
      scope.active = false;
    };
  }, [scope]);
  const isFetchingOlder = fetchingScope === scope;

  const fetchOlder = useCallback(async () => {
    if (!channelId || !scope.active || scope.fetching) {
      return;
    }
    const store =
      queryClient.getQueryData<ChannelWindowStore>(
        channelWindowKey(channelId),
      ) ?? emptyChannelWindowStore();
    if (!channelWindowHasMore(store)) {
      return;
    }

    scope.fetching = true;
    setFetchingScope(scope);
    try {
      const result = await pageOlderMessagesUntilRowFloor(
        queryClient,
        channelId,
        () => scope.active && scopeRef.current === scope,
      );
      return result.revision;
    } catch (error) {
      console.error("Failed to fetch older messages", channelId, error);
    } finally {
      scope.fetching = false;
      if (scope.active && scopeRef.current === scope) setFetchingScope(null);
    }
  }, [channelId, queryClient, scope]);

  return { fetchOlder, isFetchingOlder };
}
