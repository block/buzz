import * as React from "react";

export function useChannelTargetReset({
  activeChannelId,
  setEditTargetId,
  setExpandedThreadReplyIds,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: {
  activeChannelId: string | null;
  setEditTargetId: (id: string | null) => void;
  setExpandedThreadReplyIds: (ids: Set<string>) => void;
  setThreadReplyTargetId: (id: string | null) => void;
  setThreadScrollTargetId: (id: string | null) => void;
}) {
  React.useEffect(() => {
    // The channel identity is intentionally the reset trigger.
    void activeChannelId;
    setExpandedThreadReplyIds(new Set());
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
    setEditTargetId(null);
  }, [
    activeChannelId,
    setEditTargetId,
    setExpandedThreadReplyIds,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);
}
