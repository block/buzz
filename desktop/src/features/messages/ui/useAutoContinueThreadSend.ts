import * as React from "react";

import {
  computeAutoContinueAgentMentions,
  resolveAutoContinueAnchorId,
} from "@/features/messages/lib/autoContinueAgent";
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

  const latestMessageId = React.useMemo(() => {
    let latest: TimelineMessage | null = threadHead ?? null;
    for (const entry of threadReplies) {
      if (!latest || entry.message.createdAt >= latest.createdAt) {
        latest = entry.message;
      }
    }
    return latest?.id ?? null;
  }, [threadHead, threadReplies]);

  return React.useCallback(
    async (content, mentionPubkeys, mediaTags, channelId, threadContext) => {
      // The auto-continue anchor is the message the reply actually continues
      // from: an explicitly selected reply target, otherwise the latest message
      // in the thread. `threadContext.parentEventId` is not usable here — it
      // defaults to the thread ROOT when no reply target is selected, so the
      // agent's p-tagging message (the last reply, not the root) would never be
      // inspected and the loop would never auto-continue.
      const anchorId = resolveAutoContinueAnchorId({
        replyTargetId: replyTargetMessageRef.current?.id,
        latestMessageId,
        threadHeadId,
      });
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
      latestMessageId,
      messageById,
      onSend,
      replyTargetMessageRef,
      threadHeadId,
    ],
  );
}
