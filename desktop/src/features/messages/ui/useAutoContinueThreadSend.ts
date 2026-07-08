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
  /** Live ref to the current reply target (read at submit time). */
  replyTargetMessageRef: React.MutableRefObject<TimelineMessage | null>;
  onSend: ThreadSend;
};

/**
 * Wrap a thread `onSend` so replies auto-continue an agent's turn.
 *
 * When the user replies to a message an agent authored *and that agent
 * `p`-tagged the user*, the agent's pubkey is injected into the reply's
 * mentions even if the user did not @mention it. The injected `["p", agent]`
 * tag passes both the relay's mention-gated subscription and the ACP harness
 * `require_mention` filter, so an untagged follow-up starts a fresh agent loop
 * exactly as an explicit @mention would.
 *
 * See {@link computeAutoContinueAgentMentions} for the gating rules.
 */
export function useAutoContinueThreadSend({
  agentPubkeys,
  currentPubkey,
  threadHead,
  threadReplies,
  replyTargetMessageRef,
  onSend,
}: UseAutoContinueThreadSendOptions): ThreadSend {
  const threadHeadId = threadHead?.id ?? null;

  // Index thread messages by id so the send wrapper can resolve the reply
  // anchor (the message the reply attaches to) from the captured context.
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
