import * as React from "react";
import {
  consumePendingWelcomeInitialUnreadSuppression,
  hasPendingWelcomeInitialUnreadSuppression,
} from "@/features/onboarding/welcome";

export function initialWelcomeUnreadSuppressedChannelIds(
  activeChannelId: string | null,
  hasPendingSuppression = hasPendingWelcomeInitialUnreadSuppression,
) {
  const suppressedChannelIds = new Set<string>();
  if (activeChannelId && hasPendingSuppression(activeChannelId)) {
    suppressedChannelIds.add(activeChannelId);
  }
  return suppressedChannelIds;
}

export function addWelcomeUnreadSuppressedChannelId(
  suppressedChannelIds: ReadonlySet<string>,
  channelId: string,
) {
  if (suppressedChannelIds.has(channelId)) {
    return suppressedChannelIds;
  }

  const next = new Set(suppressedChannelIds);
  next.add(channelId);
  return next;
}

export function removeWelcomeUnreadSuppressedChannelId(
  suppressedChannelIds: ReadonlySet<string>,
  channelId: string,
) {
  if (!suppressedChannelIds.has(channelId)) {
    return suppressedChannelIds;
  }

  const next = new Set(suppressedChannelIds);
  next.delete(channelId);
  return next;
}

export function isWelcomeUnreadSuppressed(
  suppressedChannelIds: ReadonlySet<string>,
  activeChannelId: string | null,
) {
  return !!activeChannelId && suppressedChannelIds.has(activeChannelId);
}

export function useWelcomeInitialUnreadSuppression(
  activeChannelId: string | null,
  onSuppressionConsumed: () => void,
) {
  const [suppressedChannelIds, setSuppressedChannelIds] = React.useState<
    ReadonlySet<string>
  >(() => initialWelcomeUnreadSuppressedChannelIds(activeChannelId));
  const clearTimersRef = React.useRef(new Map<string, number>());
  const isMountedRef = React.useRef(false);

  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      for (const timerId of clearTimersRef.current.values()) {
        window.clearTimeout(timerId);
      }
      clearTimersRef.current.clear();
    };
  }, []);

  const cancelPendingClear = React.useCallback((channelId: string) => {
    const timerId = clearTimersRef.current.get(channelId);
    if (timerId === undefined) return;

    window.clearTimeout(timerId);
    clearTimersRef.current.delete(channelId);
  }, []);

  React.useEffect(() => {
    const channelId = activeChannelId;
    if (!channelId) return;

    cancelPendingClear(channelId);

    if (consumePendingWelcomeInitialUnreadSuppression(channelId)) {
      setSuppressedChannelIds((current) =>
        addWelcomeUnreadSuppressedChannelId(current, channelId),
      );
      onSuppressionConsumed();
    }

    return () => {
      if (!isMountedRef.current) return;

      cancelPendingClear(channelId);
      const timerId = window.setTimeout(() => {
        clearTimersRef.current.delete(channelId);
        if (!isMountedRef.current) return;

        setSuppressedChannelIds((current) =>
          removeWelcomeUnreadSuppressedChannelId(current, channelId),
        );
      }, 0);
      clearTimersRef.current.set(channelId, timerId);
    };
  }, [activeChannelId, cancelPendingClear, onSuppressionConsumed]);

  return isWelcomeUnreadSuppressed(suppressedChannelIds, activeChannelId);
}
