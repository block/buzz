import * as React from "react";

import type { PanelValueSetter } from "@/features/channels/ui/useChannelPanelHistoryState";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Keeps thread-panel and edit-composer targets consistent with the messages
 * that are actually loaded: closes the thread panel when its head message
 * disappears, seeds the reply target from the thread head, and clears stale
 * reply/edit targets that no longer resolve to a message.
 */
export function useThreadTargetSync({
  clearOptimisticThreadOverride,
  editTargetId,
  editTargetMessage,
  isTimelineLoading,
  openThreadHeadId,
  openThreadHeadMessage,
  setEditTargetId,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
  threadReplyTargetId,
  threadReplyTargetMessage,
}: {
  clearOptimisticThreadOverride: () => void;
  editTargetId: string | null;
  editTargetMessage: TimelineMessage | null;
  isTimelineLoading: boolean;
  openThreadHeadId: string | null;
  openThreadHeadMessage: TimelineMessage | null;
  setEditTargetId: (id: string | null) => void;
  setExpandedThreadReplyIds: (ids: Set<string>) => void;
  setOpenThreadHeadId: PanelValueSetter;
  setThreadReplyTargetId: (id: string | null) => void;
  setThreadScrollTargetId: (id: string | null) => void;
  threadReplyTargetId: string | null;
  threadReplyTargetMessage: TimelineMessage | null;
}) {
  React.useEffect(() => {
    if (openThreadHeadId && !openThreadHeadMessage) {
      if (isTimelineLoading) {
        return;
      }
      clearOptimisticThreadOverride();
      setOpenThreadHeadId(null, { replace: true });
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      return;
    }

    if (openThreadHeadMessage && !threadReplyTargetId) {
      setThreadReplyTargetId(openThreadHeadMessage.id);
      return;
    }

    if (threadReplyTargetId && !threadReplyTargetMessage) {
      setThreadReplyTargetId(openThreadHeadMessage?.id ?? null);
    }
    if (editTargetId && !editTargetMessage) {
      setEditTargetId(null);
    }
  }, [
    clearOptimisticThreadOverride,
    editTargetId,
    editTargetMessage,
    isTimelineLoading,
    openThreadHeadId,
    openThreadHeadMessage,
    setEditTargetId,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    threadReplyTargetId,
    threadReplyTargetMessage,
  ]);
}
