import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useCreateChatMutation,
  useSendChatContextMessageMutation,
  useUpdateChatMetadataMutation,
} from "@/features/chats/hooks";
import { deriveChatTitle } from "@/features/chats/lib/chatSetup";
import type { TimelineMessage } from "@/features/messages/types";
import { ensureWelcomeGuideAgentInChannel } from "@/features/onboarding/welcomeGuide";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import type { Channel } from "@/shared/api/types";

const SIDE_CONVERSATION_MAX_MESSAGES = 20;
const SIDE_CONVERSATION_MAX_BYTES = 16 * 1024;

function truncateToBytes(value: string, maxBytes: number) {
  const encoder = new TextEncoder();
  if (encoder.encode(value).byteLength <= maxBytes) {
    return value;
  }

  let next = value;
  while (
    next.length > 0 &&
    encoder.encode(`${next}\n\n[Truncated]`).byteLength > maxBytes
  ) {
    next = next.slice(0, -256);
  }
  return `${next.trimEnd()}\n\n[Truncated]`;
}

function buildSideConversationSnapshot({
  channelName,
  messages,
  selectedMessage,
}: {
  channelName: string;
  messages: TimelineMessage[];
  selectedMessage: TimelineMessage;
}) {
  const rootId = selectedMessage.rootId ?? selectedMessage.id;
  const threadMessages = messages.filter(
    (message) =>
      message.id === selectedMessage.id ||
      message.id === rootId ||
      message.rootId === rootId,
  );
  const boundedMessages = (
    threadMessages.length > 0 ? threadMessages : [selectedMessage]
  ).slice(-SIDE_CONVERSATION_MAX_MESSAGES);
  const body = boundedMessages
    .map((message) => {
      const author = message.author || message.pubkey?.slice(0, 8) || "Unknown";
      const timestamp = message.time ? ` at ${message.time}` : "";
      return `### ${author}${timestamp}\n\n${message.body.trim()}`;
    })
    .join("\n\n---\n\n");

  return truncateToBytes(
    `Source context from #${channelName}\n\n${body}`,
    SIDE_CONVERSATION_MAX_BYTES,
  );
}

export function useSideConversation({
  activeChannel,
  timelineMessages,
}: {
  activeChannel: Channel | null;
  timelineMessages: TimelineMessage[];
}) {
  const { goChat } = useAppNavigation();
  const { activeWorkspace } = useWorkspaces();
  const createChatMutation = useCreateChatMutation();
  const updateChatMetadataMutation = useUpdateChatMetadataMutation();
  const sendChatContextMessageMutation = useSendChatContextMessageMutation();

  return React.useCallback(
    async (message: TimelineMessage) => {
      if (!activeChannel) {
        return;
      }

      const titleSeed = message.body.trim() || activeChannel.name;
      const title = deriveChatTitle(titleSeed);
      const source = {
        channelId: activeChannel.id,
        eventId: message.id,
        threadRootId: message.rootId ?? message.id,
      };
      const snapshot = buildSideConversationSnapshot({
        channelName: activeChannel.name,
        messages: timelineMessages,
        selectedMessage: message,
      });

      try {
        const chat = await createChatMutation.mutateAsync({ title, source });
        await sendChatContextMessageMutation.mutateAsync({
          channelId: chat.id,
          content: snapshot,
          source,
        });
        const agent = await ensureWelcomeGuideAgentInChannel(
          chat.id,
          activeWorkspace?.relayUrl,
        );
        await updateChatMetadataMutation.mutateAsync({
          channelId: chat.id,
          title,
          defaultAgentPubkey: agent.pubkey,
          source,
        });
        await goChat(chat.id);
      } catch (error) {
        console.error("Failed to start side conversation", error);
        toast.error("Could not start side conversation");
      }
    },
    [
      activeChannel,
      activeWorkspace?.relayUrl,
      createChatMutation,
      goChat,
      sendChatContextMessageMutation,
      timelineMessages,
      updateChatMetadataMutation,
    ],
  );
}
