import * as React from "react";

import { markTimelineSettledThisSession } from "@/features/messages/lib/settledTimelineChannels";
import {
  abandonChannelSwitchTrace,
  markChannelSwitchRouteCommit,
  settleChannelSwitchTrace,
} from "@/shared/lib/channelSwitchPerf";
import type { ChannelType } from "@/shared/api/types";

/**
 * Switch-trace stage marks for the channel screen. Route commit fires on the
 * first render for the target channel; settle fires once its timeline leaves
 * the loading latch. Both are no-ops unless goChannel opened a trace for this
 * channel. Forum readiness is owned by ForumView's own queries, which the
 * timeline latch cannot observe — those traces are abandoned instead of
 * underreported.
 */
export function useChannelSwitchTraceMarks({
  activeChannelId,
  activeChannelType,
  isTimelineLoading,
}: {
  activeChannelId: string | null;
  activeChannelType: ChannelType | null;
  isTimelineLoading: boolean;
}): void {
  React.useEffect(() => {
    if (activeChannelId) markChannelSwitchRouteCommit(activeChannelId);
  }, [activeChannelId]);
  React.useEffect(() => {
    if (!activeChannelId) return;
    if (activeChannelType === "forum") {
      abandonChannelSwitchTrace(activeChannelId);
      return;
    }
    if (!isTimelineLoading) {
      markTimelineSettledThisSession(activeChannelId);
      settleChannelSwitchTrace(activeChannelId);
    }
  }, [activeChannelId, activeChannelType, isTimelineLoading]);
}
