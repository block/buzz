import * as React from "react";

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
  // Route-exit cancellation: leaving the channel surface before the trace
  // settles (Projects, Home, … — none of which call goChannel) must drop the
  // trace. Otherwise a history-back into the same channel within the trace
  // timeout matches the stale singleton and records the time spent away as
  // switch latency. Keyed per channel id: on an A→B switch this cleanup runs
  // with A's id after B's trace already began, so it only ever abandons its
  // own channel's trace.
  React.useEffect(() => {
    if (!activeChannelId) return;
    const channelId = activeChannelId;
    return () => {
      abandonChannelSwitchTrace(channelId);
    };
  }, [activeChannelId]);
  React.useEffect(() => {
    if (!activeChannelId) return;
    if (activeChannelType === "forum") {
      abandonChannelSwitchTrace(activeChannelId);
      return;
    }
    if (!isTimelineLoading) {
      settleChannelSwitchTrace(activeChannelId);
    }
  }, [activeChannelId, activeChannelType, isTimelineLoading]);
}
