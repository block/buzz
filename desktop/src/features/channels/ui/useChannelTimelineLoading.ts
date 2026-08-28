import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import type { Channel } from "@/shared/api/types";
import { hasPersistedHydratedChannel } from "@/features/messages/lib/channelHeadCache";
import {
  resolveTimelineLoadingLatch,
  selectTimelineLoadingState,
  type TimelineQueryStatus,
} from "@/features/messages/lib/timelineLoadingState";

type UseChannelTimelineLoadingOptions = {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  queryClient: QueryClient;
  status: TimelineQueryStatus;
};

export function useChannelTimelineLoading({
  activeChannel,
  activeChannelId,
  queryClient,
  status,
}: UseChannelTimelineLoadingOptions): boolean {
  const settledChannelIdRef = React.useRef<string | null>(null);
  const hasSettledThisChannel =
    activeChannelId !== null && settledChannelIdRef.current === activeChannelId;
  const hasHydratedTimeline =
    hasSettledThisChannel ||
    (activeChannelId !== null &&
      hasPersistedHydratedChannel(queryClient, activeChannelId));
  const loadingNow =
    activeChannel !== null &&
    activeChannel.channelType !== "forum" &&
    selectTimelineLoadingState(status, hasHydratedTimeline);
  const { settledChannelId, isLoading } = resolveTimelineLoadingLatch(
    settledChannelIdRef.current,
    activeChannelId,
    loadingNow,
  );
  settledChannelIdRef.current = settledChannelId;
  return isLoading;
}
