import * as React from "react";

import type { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { OpenAgentConversationInput } from "@/features/agents/agentConversations";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel } from "@/shared/api/types";

type GoChannel = ReturnType<typeof useAppNavigation>["goChannel"];
type OpenAgentConversation = (
  input: OpenAgentConversationInput,
  options?: { publishMarker?: boolean },
) => void;

type UseAgentConversationRouteTargetOptions = {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  goChannel: GoChannel;
  messageProfilesReady: boolean;
  openAgentConversation: OpenAgentConversation;
  targetAgentConversationReplyId: string | null;
  timelineMessages: readonly TimelineMessage[];
};

export function useAgentConversationRouteTarget({
  activeChannel,
  activeChannelId,
  goChannel,
  messageProfilesReady,
  openAgentConversation,
  targetAgentConversationReplyId,
  timelineMessages,
}: UseAgentConversationRouteTargetOptions) {
  const handledRouteTargetRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!targetAgentConversationReplyId) {
      handledRouteTargetRef.current = null;
      return;
    }

    const targetKey = `${activeChannelId ?? "none"}:${targetAgentConversationReplyId}`;
    if (handledRouteTargetRef.current === targetKey) {
      return;
    }
    if (!activeChannel || activeChannel.channelType === "forum") {
      return;
    }
    if (!messageProfilesReady) {
      return;
    }

    const agentReply =
      timelineMessages.find(
        (message) => message.id === targetAgentConversationReplyId,
      ) ?? null;
    const agentReplyPubkey = agentReply?.pubkey;
    if (!agentReply || !agentReplyPubkey) {
      return;
    }

    const rootId = agentReply.rootId ?? agentReply.parentId ?? agentReply.id;
    const contextMessages = timelineMessages.filter(
      (candidate) =>
        candidate.id === rootId ||
        candidate.id === agentReply.id ||
        candidate.rootId === rootId ||
        candidate.parentId === rootId,
    );
    const parentMessage = agentReply.parentId
      ? (timelineMessages.find(
          (candidate) => candidate.id === agentReply.parentId,
        ) ?? null)
      : null;
    const threadRootMessage =
      timelineMessages.find((candidate) => candidate.id === rootId) ?? null;

    handledRouteTargetRef.current = targetKey;
    void goChannel(activeChannel.id, { replace: true }).then(() => {
      openAgentConversation(
        {
          agentName: agentReply.author,
          agentPubkey: agentReplyPubkey,
          agentReply,
          channel: activeChannel,
          contextMessages,
          parentMessage,
          threadRootMessage,
        },
        { publishMarker: false },
      );
    });
  }, [
    activeChannel,
    activeChannelId,
    goChannel,
    messageProfilesReady,
    openAgentConversation,
    targetAgentConversationReplyId,
    timelineMessages,
  ]);
}
