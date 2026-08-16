import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { collectRenderScopedReactionMessageIds } from "./renderScopedReactions";
import { hydrateRenderScopedDispositions } from "./renderScopedDispositions";
import type { MainTimelineEntry } from "./threadPanel";
import type { TimelineMessage } from "../types";
import type { Channel } from "@/shared/api/types";

/**
 * Mirrors {@link useRenderScopedReactionHydration} exactly — same visible-id
 * collection (reused as-is; it's kind-agnostic despite the reaction-specific
 * name), same debounce shape — but fetches NIP-AD dispositions instead of
 * reactions. Kept as a separate hook/effect rather than folded into the
 * reaction one so a disposition-hydration bug can never regress reactions.
 */
export function useRenderScopedDispositionHydration(input: {
  activeChannel: Channel | null;
  mainTimelineEntries: MainTimelineEntry[];
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
}) {
  const queryClient = useQueryClient();

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
      void hydrateRenderScopedDispositions({
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
