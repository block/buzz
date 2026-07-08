import * as React from "react";

import { computeAutoContinueAgentMentions } from "@/features/messages/lib/autoContinueAgent";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

type ThreadContext = {
  parentEventId: string | null;
  threadHeadId: string | null;
} | null;

type ThreadSend = (
  content: string,
  mentionPubkeys: string[],
  mediaTags?: string[][],
  channelId?: string | null,
  threadContext?: ThreadContext,
) => Promise<void>;

type UseAutoContinueThreadSendOptions = {
  agentPubkeys?: ReadonlySet<string>;
  currentPubkey?: string;
  threadHead: TimelineMessage | null;
  threadReplies: MainTimelineEntry[];
  replyTargetMessageRef: React.MutableRefObject<TimelineMessage | null>;
  onSend: ThreadSend;
};

export function useAutoContinueThreadSend({
  agentPubkeys,
  currentPubkey,
  threadHead,
  threadReplies,
  replyTargetMessageRef,
  onSend,
}: UseAutoContinueThreadSendOptions): ThreadSend {
  const threadHeadId = threadHead?.id ?? null;

  const messageById = React.useMemo(() => {
    const index = new Map<string, TimelineMessage>();
    if (threadHead) {
      index.set(threadHead.id, threadHead);
    }
    for (const entry of threadReplies) {
      index.set(entry.message.id, entry.message);
    }
    return index;
  }, [threadHead, threadReplies]);

  return React.useCallback(
    async (content, mentionPubkeys, mediaTags, channelId, threadContext) => {
      const anchorId =
        threadContext?.parentEventId ??
        replyTargetMessageRef.current?.id ??
        threadHeadId;
      const anchor = anchorId ? messageById.get(anchorId) : null;
      const autoMentions = computeAutoContinueAgentMentions({
        anchor,
        currentPubkey,
        agentPubkeys,
        existingMentionPubkeys: mentionPubkeys,
      });
      const effectiveMentions =
        autoMentions.length > 0
          ? [...mentionPubkeys, ...autoMentions]
          : mentionPubkeys;
      await onSend(
        content,
        effectiveMentions,
        mediaTags,
        channelId,
        threadContext,
      );
    },
    [
      agentPubkeys,
      currentPubkey,
      messageById,
      onSend,
      replyTargetMessageRef,
      threadHeadId,
    ],
  );
}
