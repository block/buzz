import { useCallback, useEffect, useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelMessagesQueryOptions } from "@/features/messages/hooks";
import {
  bindChannelIntentLifecycle,
  createChannelIntentEventHandlers,
  createChannelIntentScheduler,
  resolveIntentChannel,
} from "@/features/sidebar/lib/channelIntentPrefetch";
import type { Channel } from "@/shared/api/types";

export function useChannelIntentPrefetch(
  channels: Channel[],
  selectedChannelId: string | null,
) {
  const queryClient = useQueryClient();
  const prefetch = useCallback(
    (channel: Channel) => {
      void queryClient.prefetchQuery(
        channelMessagesQueryOptions(queryClient, channel),
      );
    },
    [queryClient],
  );
  const scheduler = useMemo(
    () => createChannelIntentScheduler(prefetch),
    [prefetch],
  );
  useEffect(() => bindChannelIntentLifecycle(scheduler), [scheduler]);

  const channelById = useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: channel set and selection define the pending intent boundary
  useEffect(() => {
    // A pending candidate belongs to this exact channel set and selection.
    scheduler.clear();
  }, [channelById, scheduler, selectedChannelId]);
  const resolveChannel = useCallback(
    (channelId: string) =>
      resolveIntentChannel(channelById, selectedChannelId, channelId),
    [channelById, selectedChannelId],
  );

  return useMemo(
    () => createChannelIntentEventHandlers(resolveChannel, scheduler),
    [resolveChannel, scheduler],
  );
}
