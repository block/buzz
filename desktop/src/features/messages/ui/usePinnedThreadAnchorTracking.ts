import * as React from "react";

import { useThreadRailContext } from "@/features/channels/ThreadRailContext";
import { getThreadRailAnchorUpdate } from "@/features/channels/threadRailAnchor";
import type { TimelineMessage } from "@/features/messages/types";

export function usePinnedThreadAnchorTracking({
  channelId,
  onSelectReplyTarget,
  threadHeadId,
}: {
  channelId: string | null;
  onSelectReplyTarget: (message: TimelineMessage) => void;
  threadHeadId: string | null;
}) {
  const threadRail = useThreadRailContext();

  return React.useCallback(
    (message: TimelineMessage) => {
      const update = getThreadRailAnchorUpdate(
        threadRail.pins,
        channelId,
        threadHeadId,
        // Rows rendered here belong to the current canonical thread even when
        // a locally expanded TimelineMessage omits rootId.
        { ...message, rootId: threadHeadId },
      );
      if (update) {
        threadRail.updateAnchor(update.pin, update.returnAnchorId);
      }
      onSelectReplyTarget(message);
    },
    [channelId, onSelectReplyTarget, threadHeadId, threadRail],
  );
}
