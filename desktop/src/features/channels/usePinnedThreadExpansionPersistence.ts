import * as React from "react";

import type { ThreadRail } from "@/features/channels/useThreadRail";

type PinnedThreadExpansionParams = {
  activeChannelId: string | null;
  effectiveOpenThreadHeadId: string | null;
  threadRail: ThreadRail;
};

export function usePinnedThreadExpandedReplyIdsChange({
  activeChannelId,
  effectiveOpenThreadHeadId,
  threadRail,
}: PinnedThreadExpansionParams) {
  return React.useCallback(
    (expandedReplyIds: ReadonlySet<string>) => {
      if (!activeChannelId || !effectiveOpenThreadHeadId) return;
      const pin = threadRail.pins.find(
        (candidate) =>
          candidate.channelId === activeChannelId &&
          candidate.rootId === effectiveOpenThreadHeadId,
      );
      if (pin) {
        threadRail.updateExpandedReplyIds(pin, [...expandedReplyIds]);
      }
    },
    [activeChannelId, effectiveOpenThreadHeadId, threadRail],
  );
}

export function useRestorePinnedThreadExpansion({
  activeChannelId,
  effectiveOpenThreadHeadId,
  setExpandedThreadReplyIds,
  threadRail,
  threadRouteTargetReady,
}: PinnedThreadExpansionParams & {
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  threadRouteTargetReady: boolean;
}) {
  const restoredPinKeyRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!threadRouteTargetReady) return;
    const pin = threadRail.pins.find(
      (candidate) =>
        candidate.channelId === activeChannelId &&
        candidate.rootId === effectiveOpenThreadHeadId,
    );
    if (!pin) {
      restoredPinKeyRef.current = null;
      return;
    }

    const pinKey = `${pin.channelId}\u0000${pin.rootId}`;
    if (restoredPinKeyRef.current === pinKey) return;

    // Register this after the generic thread-reset hooks: a pinned destination
    // restores only after ordinary channel navigation has cleared expansion.
    restoredPinKeyRef.current = pinKey;
    setExpandedThreadReplyIds(
      (current) => new Set([...current, ...(pin.expandedReplyIds ?? [])]),
    );
  }, [
    activeChannelId,
    effectiveOpenThreadHeadId,
    setExpandedThreadReplyIds,
    threadRail.pins,
    threadRouteTargetReady,
  ]);
}
