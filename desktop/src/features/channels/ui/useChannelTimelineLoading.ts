import * as React from "react";
import {
  resolveTimelineLoadingLatch,
  selectTimelineLoadingState,
} from "@/features/messages/lib/timelineLoadingState";
import type { Channel, RelayEvent } from "@/shared/api/types";

type ChannelMessagesQueryState = {
  data?: RelayEvent[];
  isFetching: boolean;
  isPending: boolean;
  isPlaceholderData: boolean;
};

export function useChannelTimelineLoading(
  activeChannel: Channel | null,
  messagesQuery: ChannelMessagesQueryState,
): boolean {
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
      hasSettledThisChannel,
    );
  const { settledChannelId, isLoading } = resolveTimelineLoadingLatch(
    settledChannelIdRef.current,
    activeChannelId,
    timelineLoadingNow,
  );
  settledChannelIdRef.current = settledChannelId;
  return isLoading;
}
