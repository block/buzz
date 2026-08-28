import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { hasPersistedHydratedChannel } from "@/features/messages/lib/channelHeadCache";
import {
  resolveTimelineLoadingLatch,
  selectTimelineLoadingState,
} from "@/features/messages/lib/timelineLoadingState";
import type { Channel } from "@/shared/api/types";

/**
 * Latches the timeline loading state per channel, so a channel that has
 * already settled once does not flash its skeleton again while an
 * authoritative refresh is in flight.
 */
export function useChannelTimelineLoading(
  activeChannel: Channel | null,
  messagesQuery: {
    data: readonly unknown[] | undefined;
    isFetching: boolean;
    isPending: boolean;
    isPlaceholderData: boolean;
  },
): boolean {
  const queryClient = useQueryClient();
  const activeChannelId = activeChannel?.id ?? null;
  const settledChannelIdRef = React.useRef<string | null>(null);
  const hasSettledThisChannel =
    activeChannelId !== null && settledChannelIdRef.current === activeChannelId;
  const timelineLoadingNow =
    activeChannel !== null &&
    activeChannel.channelType !== "forum" &&
    selectTimelineLoadingState(
      {
        isPending: messagesQuery.isPending,
        isFetching: messagesQuery.isFetching,
        isPlaceholderData: messagesQuery.isPlaceholderData,
        dataLength: messagesQuery.data?.length ?? null,
      },
      // A persisted head only counts as hydrated when it has rows to paint
      // (channelHeadCache.ts), so this bypass never settles onto an empty
      // placeholder while the authoritative refresh is still in flight.
      hasSettledThisChannel ||
        (activeChannelId !== null &&
          hasPersistedHydratedChannel(queryClient, activeChannelId)),
    );
  const { settledChannelId, isLoading: isTimelineLoading } =
    resolveTimelineLoadingLatch(
      settledChannelIdRef.current,
      activeChannelId,
      timelineLoadingNow,
    );
  settledChannelIdRef.current = settledChannelId;
  return isTimelineLoading;
}
