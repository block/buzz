import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { MainTimelineEntry } from "./threadPanel";
import type { TimelineMessage } from "../types";
import type { Channel } from "@/shared/api/types";
import {
  collectRenderScopedReactionMessageIds,
  hydrateRenderScopedReactions,
  releaseDeadlineClaimedReactionIds,
} from "./renderScopedReactions";

export function useRenderScopedReactionHydration(input: {
  activeChannel: Channel | null;
  mainTimelineEntries: MainTimelineEntry[];
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
}) {
  const queryClient = useQueryClient();
  const channelId = input.activeChannel?.id;

  // Opening a channel is the explicit retry for reactions a relay deadline
  // held back. Declared first so it runs before this render's hydration.
  React.useEffect(() => {
    if (channelId) releaseDeadlineClaimedReactionIds(channelId);
  }, [channelId]);

  React.useEffect(() => {
    const channelId = input.activeChannel?.id;
    if (!channelId || input.activeChannel?.channelType === "forum") {
      return;
    }

    const messageIds = collectRenderScopedReactionMessageIds({
      mainEntries: input.mainTimelineEntries,
      threadHeadMessage: input.threadHeadMessage,
      threadEntries: input.threadMessages,
    });
    if (messageIds.length === 0) return;

    const timeout = window.setTimeout(() => {
      void hydrateRenderScopedReactions({
        channelId,
        messageIds,
        queryClient,
      });
    }, 0);

    return () => window.clearTimeout(timeout);
  }, [
    input.activeChannel,
    input.mainTimelineEntries,
    input.threadHeadMessage,
    input.threadMessages,
    queryClient,
  ]);
}
