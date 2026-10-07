import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { TimelineMessage } from "@/features/messages/types";

type ThreadSendContext = {
  parentEventId: string | null;
  threadHeadId: string | null;
};

export function useProjectedThreadComposer({
  activeChannelId,
  onOpenThread,
  threadRepliesInChannel,
}: {
  activeChannelId: string | null;
  onOpenThread?: (message: TimelineMessage) => void;
  threadRepliesInChannel: boolean;
}) {
  const { goChannel } = useAppNavigation();
  const [target, setTarget] = React.useState<TimelineMessage | null>(null);
  const scopeRef = React.useRef("");
  const rootId = target ? (target.rootId ?? target.parentId ?? null) : null;
  const composerTarget = React.useMemo(
    () =>
      target && rootId
        ? {
            author: target.author,
            body: target.body,
            id: target.id,
          }
        : null,
    [rootId, target],
  );
  const scopeKey = `${activeChannelId ?? ""}:${threadRepliesInChannel ? "thread-replies-in-channel" : "panel"}`;

  React.useEffect(() => {
    if (scopeRef.current === scopeKey) return;
    scopeRef.current = scopeKey;
    setTarget(null);
  }, [scopeKey]);

  const openProjectedThread = React.useCallback(
    (message: TimelineMessage) => {
      const projectedRootId = message.rootId ?? message.parentId ?? null;
      if (!activeChannelId || !projectedRootId) {
        onOpenThread?.(message);
        return;
      }
      void goChannel(activeChannelId, {
        force: true,
        messageId: projectedRootId,
        threadRootId: projectedRootId,
      });
    },
    [activeChannelId, goChannel, onOpenThread],
  );

  const replyToProjectedThread = React.useCallback(
    (message: TimelineMessage) => {
      if (!message.parentId) {
        onOpenThread?.(message);
        return;
      }
      setTarget((current) => (current?.id === message.id ? null : message));
    },
    [onOpenThread],
  );

  const cancelReply = React.useCallback(() => {
    setTarget(null);
  }, []);

  const captureSendContext = React.useCallback((): ThreadSendContext | null => {
    if (!target || !rootId) return null;
    return {
      parentEventId: target.id,
      threadHeadId: rootId,
    };
  }, [rootId, target]);

  const clearSentTarget = React.useCallback((parentEventId: string | null) => {
    if (!parentEventId) return;
    setTarget((current) => (current?.id === parentEventId ? null : current));
  }, []);

  return React.useMemo(
    () => ({
      cancelReply,
      captureSendContext,
      clearSentTarget,
      composerTarget,
      openProjectedThread,
      replyToProjectedThread,
      rootId,
      target,
    }),
    [
      cancelReply,
      captureSendContext,
      clearSentTarget,
      composerTarget,
      openProjectedThread,
      replyToProjectedThread,
      rootId,
      target,
    ],
  );
}
