import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { isBroadcastReply } from "@/features/messages/lib/threading";
import type { Channel } from "@/shared/api/types";
import type { PanelValueSetter } from "./useChannelPanelHistoryState";

function getThreadRouteTarget(
  targetMessage: TimelineMessage,
  messageById: ReadonlyMap<string, TimelineMessage>,
): { expandedReplyIds: Set<string>; threadHeadId: string } | null {
  const threadHeadId = targetMessage.rootId ?? targetMessage.parentId ?? null;
  if (!threadHeadId || !messageById.has(threadHeadId)) {
    return null;
  }

  const expandedReplyIds = new Set<string>();
  let ancestorId = targetMessage.parentId ?? null;
  let guard = 0;
  const maxHops = messageById.size + 1;

  while (ancestorId && ancestorId !== threadHeadId && guard < maxHops) {
    const ancestor = messageById.get(ancestorId);
    if (!ancestor) {
      return null;
    }

    expandedReplyIds.add(ancestor.id);
    ancestorId = ancestor.parentId ?? null;
    guard += 1;
  }

  if (ancestorId !== threadHeadId) {
    return null;
  }

  return { expandedReplyIds, threadHeadId };
}

function getRouteMainTimelineTargetId(
  targetMessageId: string | null,
  targetMessage: TimelineMessage | null,
): string | null {
  if (!targetMessageId) {
    return null;
  }

  if (!targetMessage) {
    return null;
  }

  if (!targetMessage.parentId || isBroadcastReply(targetMessage.tags ?? [])) {
    return targetMessageId;
  }

  return targetMessage.rootId ?? targetMessage.parentId;
}

export function useChannelRouteTarget({
  activeChannel,
  activeChannelId,
  closeAgentSession,
  setEditTargetId,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
  targetMessageId,
  timelineMessages,
}: {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  closeAgentSession: () => void;
  setEditTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setOpenThreadHeadId: PanelValueSetter;
  setProfilePanelPubkey: PanelValueSetter;
  setThreadReplyTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  setThreadScrollTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  targetMessageId: string | null;
  timelineMessages: TimelineMessage[];
}) {
  const timelineMessageById = React.useMemo(
    () => new Map(timelineMessages.map((message) => [message.id, message])),
    [timelineMessages],
  );
  const targetTimelineMessage = targetMessageId
    ? (timelineMessageById.get(targetMessageId) ?? null)
    : null;
  const mainTimelineTargetMessageId = getRouteMainTimelineTargetId(
    targetMessageId,
    targetTimelineMessage,
  );
  const rootThreadHeadTargetId =
    targetTimelineMessage && !targetTimelineMessage.parentId
      ? targetTimelineMessage.id
      : null;
  const handledThreadRouteTargetRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!targetMessageId) {
      handledThreadRouteTargetRef.current = null;
      return;
    }

    const targetKey = `${activeChannelId ?? "none"}:${targetMessageId}`;
    if (handledThreadRouteTargetRef.current !== targetKey) {
      handledThreadRouteTargetRef.current = null;
    }

    if (
      handledThreadRouteTargetRef.current === targetKey ||
      !activeChannel ||
      activeChannel.channelType === "forum"
    ) {
      return;
    }

    const targetMessage = timelineMessageById.get(targetMessageId) ?? null;
    if (!targetMessage) {
      return;
    }

    if (!targetMessage.parentId) {
      // Root links still need to open the reply panel, but not until the main
      // timeline has centered the row. Opening the panel first changes the main
      // column layout mid-jump and can make the virtualized target abandon early.
      return;
    }

    if (isBroadcastReply(targetMessage.tags ?? [])) {
      return;
    }

    const routeTarget = getThreadRouteTarget(
      targetMessage,
      timelineMessageById,
    );
    if (!routeTarget) {
      return;
    }

    closeAgentSession();
    // Replace so the deep-link entry itself carries the opened thread —
    // back should leave the deep link, not strip the panel from it.
    setProfilePanelPubkey(null, { replace: true });
    setEditTargetId(null);
    setOpenThreadHeadId(routeTarget.threadHeadId, { replace: true });
    setThreadReplyTargetId(routeTarget.threadHeadId);
    setThreadScrollTargetId(targetMessageId);
    setExpandedThreadReplyIds(routeTarget.expandedReplyIds);
    handledThreadRouteTargetRef.current = targetKey;
  }, [
    activeChannel,
    activeChannelId,
    closeAgentSession,
    setEditTargetId,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    targetMessageId,
    timelineMessageById,
  ]);

  return { mainTimelineTargetMessageId, rootThreadHeadTargetId };
}
