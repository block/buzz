import * as React from "react";
import {
  consumePendingWelcomeInitialUnreadSuppression,
  hasPendingWelcomeInitialUnreadSuppression,
} from "@/features/onboarding/welcome";

export function useWelcomeInitialUnreadSuppression(
  activeChannelId: string | null,
  onSuppressionConsumed: () => void,
) {
  const suppressedChannelIdsRef = React.useRef(new Set<string>());

  if (
    activeChannelId &&
    hasPendingWelcomeInitialUnreadSuppression(activeChannelId)
  ) {
    suppressedChannelIdsRef.current.add(activeChannelId);
  }

  React.useEffect(() => {
    const channelId = activeChannelId;
    if (!channelId) return;

    if (consumePendingWelcomeInitialUnreadSuppression(channelId)) {
      suppressedChannelIdsRef.current.add(channelId);
      onSuppressionConsumed();
    }

    return () => {
      suppressedChannelIdsRef.current.delete(channelId);
    };
  }, [activeChannelId, onSuppressionConsumed]);

  return (
    !!activeChannelId && suppressedChannelIdsRef.current.has(activeChannelId)
  );
}
