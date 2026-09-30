import * as React from "react";
import { toast } from "sonner";
import { isThreadReply } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

type Input = {
  activeChannelId: string | null;
  channelIsCovered: boolean;
  currentPubkey?: string | null;
  editTarget: { id: string; isThreadReply: boolean } | null;
  isSinglePanelView: boolean;
  mainMessages: TimelineMessage[];
  onCloseThread: () => void;
  onEdit?: (message: TimelineMessage) => void;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: TimelineMessage[];
  useFocusThreadDrawer: boolean;
};

export function useRoutedMessageEdit({
  activeChannelId,
  channelIsCovered,
  currentPubkey,
  editTarget,
  isSinglePanelView,
  mainMessages,
  onCloseThread,
  onEdit,
  threadHeadMessage,
  threadMessages,
  useFocusThreadDrawer,
}: Input) {
  const pendingMainEditRef = React.useRef<TimelineMessage | null>(null);
  const editTargetRef = React.useRef(editTarget);
  editTargetRef.current = editTarget;
  const contextRef = React.useRef({
    channelId: activeChannelId,
    threadId: threadHeadMessage?.id,
  });
  const context = {
    channelId: activeChannelId,
    threadId: threadHeadMessage?.id,
  };
  if (
    contextRef.current.channelId !== context.channelId ||
    (contextRef.current.threadId &&
      context.threadId &&
      contextRef.current.threadId !== context.threadId)
  )
    pendingMainEditRef.current = null;
  contextRef.current = context;

  const findLastOwnEditable = React.useCallback(
    (messages: TimelineMessage[]) => {
      if (!onEdit || !currentPubkey) return null;
      return messages.reduce<TimelineMessage | null>(
        (best, message) =>
          message.kind === KIND_SYSTEM_MESSAGE ||
          message.pubkey !== currentPubkey ||
          message.pending ||
          (best && message.createdAt < best.createdAt)
            ? best
            : message,
        null,
      );
    },
    [currentPubkey, onEdit],
  );
  const routeEdit = React.useCallback(
    (message: TimelineMessage) => {
      const current = editTargetRef.current;
      if (
        current &&
        current.id !== message.id &&
        current.isThreadReply !== isThreadReply(message.tags ?? [])
      ) {
        pendingMainEditRef.current = null;
        toast.info("Finish or cancel your edit first.");
        return false;
      }
      if (current?.id === message.id) {
        pendingMainEditRef.current = null;
        onEdit?.(message);
        return true;
      }
      if (
        !isThreadReply(message.tags ?? []) &&
        (isSinglePanelView || useFocusThreadDrawer)
      ) {
        pendingMainEditRef.current = message;
        onCloseThread();
        return true;
      }
      onEdit?.(message);
      return Boolean(onEdit);
    },
    [isSinglePanelView, onCloseThread, onEdit, useFocusThreadDrawer],
  );
  const handleEditLastOwnMainMessage = React.useCallback(() => {
    const target = findLastOwnEditable(mainMessages);
    return target ? routeEdit(target) : false;
  }, [findLastOwnEditable, mainMessages, routeEdit]);
  const handleEditLastOwnThreadMessage = React.useCallback(() => {
    const target = findLastOwnEditable(
      threadHeadMessage
        ? [threadHeadMessage, ...threadMessages]
        : threadMessages,
    );
    return target ? routeEdit(target) : false;
  }, [findLastOwnEditable, routeEdit, threadHeadMessage, threadMessages]);
  React.useEffect(() => {
    const pending = pendingMainEditRef.current;
    if (!pending || isSinglePanelView || channelIsCovered) return;
    pendingMainEditRef.current = null;
    onEdit?.(pending);
  }, [channelIsCovered, isSinglePanelView, onEdit]);
  return {
    handleEditLastOwnMainMessage,
    handleEditLastOwnThreadMessage,
    routeEdit,
  };
}
