import * as React from "react";

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

type UseLastOwnMessageEditOptions = {
  currentPubkey?: string;
  messages: TimelineMessage[];
  onEdit?: (message: TimelineMessage) => void;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
};

export function useLastOwnMessageEdit({
  currentPubkey,
  messages,
  onEdit,
  threadHeadMessage,
  threadMessages,
}: UseLastOwnMessageEditOptions) {
  const findLastOwnEditable = React.useCallback(
    (candidates: TimelineMessage[]): TimelineMessage | null => {
      if (!onEdit || !currentPubkey) return null;
      let best: TimelineMessage | null = null;
      for (const message of candidates) {
        if (
          message.kind === KIND_SYSTEM_MESSAGE ||
          message.pubkey !== currentPubkey ||
          message.pending
        ) {
          continue;
        }
        if (!best || message.createdAt >= best.createdAt) {
          best = message;
        }
      }
      return best;
    },
    [currentPubkey, onEdit],
  );

  const handleEditLastOwnMainMessage = React.useCallback((): boolean => {
    const target = findLastOwnEditable(messages);
    if (!target || !onEdit) return false;
    onEdit(target);
    return true;
  }, [findLastOwnEditable, messages, onEdit]);

  const handleEditLastOwnThreadMessage = React.useCallback((): boolean => {
    if (!onEdit) return false;
    const scope: TimelineMessage[] = [];
    if (threadHeadMessage) scope.push(threadHeadMessage);
    for (const entry of threadMessages) scope.push(entry.message);
    const target = findLastOwnEditable(scope);
    if (!target) return false;
    onEdit(target);
    return true;
  }, [findLastOwnEditable, onEdit, threadHeadMessage, threadMessages]);

  return { handleEditLastOwnMainMessage, handleEditLastOwnThreadMessage };
}
