import * as React from "react";

import type {
  AgentConversationMarker,
  OpenAgentConversationInput,
} from "@/features/agents/agentConversations";
import { collectMessageMentionPubkeys } from "@/features/messages/lib/formatTimelineMessages";
import type { TimelineMessage } from "@/features/messages/types";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type GoChannel = (
  channelId: string,
  options?: {
    messageId?: string;
    replace?: boolean;
    taskReplyId?: string;
    threadRootId?: string | null;
  },
) => Promise<boolean>;

type UseAgentConversationRouteTargetInput = {
  activeChannel: Channel | null;
  agentConversationMarkers: readonly AgentConversationMarker[];
  agentPubkeys: ReadonlySet<string>;
  agentLookupReady: boolean;
  enabled: boolean;
  goChannel: GoChannel;
  messageProfilesReady: boolean;
  openAgentConversation: (
    input: OpenAgentConversationInput,
    options?: { publishMarker?: boolean },
  ) => void;
  targetAgentConversationReplyId: string | null;
  timelineMessages: readonly TimelineMessage[];
};

function getSingleDmAgentPubkey(
  channel: Channel,
  agentPubkeys: ReadonlySet<string>,
) {
  if (channel.channelType !== "dm") {
    return "";
  }

  const dmAgentPubkeys = new Map<string, string>();
  for (const pubkey of channel.participantPubkeys) {
    const normalized = normalizePubkey(pubkey);
    if (agentPubkeys.has(normalized)) {
      dmAgentPubkeys.set(normalized, pubkey);
    }
  }

  return dmAgentPubkeys.size === 1 ? [...dmAgentPubkeys.values()][0] : "";
}

export function useAgentConversationRouteTarget({
  activeChannel,
  agentConversationMarkers,
  agentLookupReady,
  agentPubkeys,
  enabled,
  goChannel,
  messageProfilesReady,
  openAgentConversation,
  targetAgentConversationReplyId,
  timelineMessages,
}: UseAgentConversationRouteTargetInput) {
  const handledRouteTargetRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!enabled || !targetAgentConversationReplyId) {
      handledRouteTargetRef.current = null;
      return;
    }

    const targetKey = `${activeChannel?.id ?? "none"}:${targetAgentConversationReplyId}`;
    if (handledRouteTargetRef.current === targetKey) {
      return;
    }
    if (!activeChannel || activeChannel.channelType === "forum") {
      return;
    }
    if (!messageProfilesReady) {
      return;
    }

    const marker =
      agentConversationMarkers.find(
        (candidate) =>
          candidate.channelId === activeChannel.id &&
          candidate.agentReplyId === targetAgentConversationReplyId,
      ) ?? null;
    const sourceMessage =
      timelineMessages.find(
        (message) => message.id === targetAgentConversationReplyId,
      ) ?? null;
    if (!sourceMessage) {
      return;
    }
    if (!marker && !agentLookupReady) {
      return;
    }

    const sourceAuthorIsAgent = sourceMessage.pubkey
      ? agentPubkeys.has(normalizePubkey(sourceMessage.pubkey))
      : false;
    const mentionedAgentPubkey =
      collectMessageMentionPubkeys([sourceMessage]).find((pubkey) =>
        agentPubkeys.has(normalizePubkey(pubkey)),
      ) ?? "";
    const dmAgentPubkey = getSingleDmAgentPubkey(activeChannel, agentPubkeys);
    const taskAgentPubkey =
      marker?.agentPubkey ||
      (sourceAuthorIsAgent ? (sourceMessage.pubkey ?? "") : "") ||
      mentionedAgentPubkey ||
      dmAgentPubkey;
    const taskAgentName =
      marker?.agentName ||
      (sourceAuthorIsAgent && taskAgentPubkey ? sourceMessage.author : "") ||
      taskAgentPubkey;
    const rootId =
      sourceMessage.rootId ?? sourceMessage.parentId ?? sourceMessage.id;
    const contextMessages = timelineMessages.filter(
      (candidate) =>
        candidate.id === rootId ||
        candidate.id === sourceMessage.id ||
        candidate.rootId === rootId ||
        candidate.parentId === rootId,
    );
    const parentMessage = sourceMessage.parentId
      ? (timelineMessages.find(
          (candidate) => candidate.id === sourceMessage.parentId,
        ) ?? null)
      : null;
    const threadRootMessage =
      timelineMessages.find((candidate) => candidate.id === rootId) ?? null;

    handledRouteTargetRef.current = targetKey;
    void goChannel(activeChannel.id, { replace: true }).then(() => {
      openAgentConversation(
        {
          agentName: taskAgentName,
          agentPubkey: taskAgentPubkey,
          agentReply: sourceMessage,
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
    agentConversationMarkers,
    agentLookupReady,
    agentPubkeys,
    enabled,
    goChannel,
    messageProfilesReady,
    openAgentConversation,
    targetAgentConversationReplyId,
    timelineMessages,
  ]);
}
