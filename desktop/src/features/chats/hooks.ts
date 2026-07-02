import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { archiveChannel } from "@/shared/api/tauri";
import {
  createChat,
  getChatMetadata,
  listChatMetadata,
  listChats,
  sendChatContextMessage,
  updateChatMetadata,
} from "@/shared/api/tauriChats";
import type {
  Channel,
  ChatMetadata,
  CreateChatInput,
  SendChatContextMessageInput,
  UpdateChatMetadataInput,
} from "@/shared/api/types";

export const chatsQueryKey = ["chats"] as const;
export const chatMetadataListQueryKey = ["chats", "metadata"] as const;
export const chatMetadataQueryKey = (channelId: string) =>
  ["chats", channelId, "metadata"] as const;

function sortChats(chats: Channel[]) {
  return [...chats].sort((left, right) => {
    const leftTime = chatActivityTime(left);
    const rightTime = chatActivityTime(right);
    if (leftTime !== rightTime) {
      return rightTime - leftTime;
    }
    return left.name.localeCompare(right.name);
  });
}

function chatActivityTime(chat: Channel) {
  const timestamp = chat.lastMessageAt ?? chat.createdAt;
  return timestamp ? Date.parse(timestamp) : 0;
}

export function useChatsQuery() {
  return useQuery({
    queryKey: chatsQueryKey,
    queryFn: async () =>
      sortChats((await listChats()).filter((chat) => chat.archivedAt === null)),
    staleTime: 60_000,
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });
}

export function useArchiveChatMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: archiveChannel,
    onMutate: async (channelId: string) => {
      await Promise.all([
        queryClient.cancelQueries({ queryKey: chatsQueryKey }),
        queryClient.cancelQueries({ queryKey: channelsQueryKey }),
      ]);
      const previousChats = queryClient.getQueryData<Channel[]>(chatsQueryKey);
      const previousChannels =
        queryClient.getQueryData<Channel[]>(channelsQueryKey);

      queryClient.setQueryData<Channel[]>(chatsQueryKey, (current = []) =>
        current.filter((chat) => chat.id !== channelId),
      );
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) =>
        current.map((channel) =>
          channel.id === channelId
            ? { ...channel, archivedAt: new Date().toISOString() }
            : channel,
        ),
      );

      return { previousChats, previousChannels };
    },
    onError: (_error, _channelId, context) => {
      if (context?.previousChats) {
        queryClient.setQueryData(chatsQueryKey, context.previousChats);
      }
      if (context?.previousChannels) {
        queryClient.setQueryData(channelsQueryKey, context.previousChannels);
      }
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: chatsQueryKey }),
        queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
      ]);
    },
  });
}

export function useChatMetadataQuery(channelId: string | null | undefined) {
  return useQuery({
    enabled: Boolean(channelId),
    queryKey: channelId ? chatMetadataQueryKey(channelId) : ["chats", "none"],
    queryFn: () => getChatMetadata(channelId ?? ""),
    staleTime: 60_000,
  });
}

export function useChatMetadataListQuery() {
  return useQuery({
    queryKey: chatMetadataListQueryKey,
    queryFn: listChatMetadata,
    staleTime: 60_000,
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
  });
}

export function useCreateChatMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: CreateChatInput) => createChat(input),
    onSuccess: (chat, input) => {
      queryClient.setQueryData<Channel[]>(chatsQueryKey, (current = []) =>
        sortChats([
          ...current.filter((channel) => channel.id !== chat.id),
          chat,
        ]),
      );
      queryClient.setQueryData<ChatMetadata[]>(
        chatMetadataListQueryKey,
        (current = []) => [
          ...current.filter((metadata) => metadata.channelId !== chat.id),
          {
            channelId: chat.id,
            authorPubkey: null,
            title: input.title ?? chat.name,
            defaultAgentPubkey: input.defaultAgentPubkey ?? null,
            templateId: input.templateId ?? null,
            projectId: input.projectId ?? null,
            projectName: input.projectName ?? null,
            projectPath: input.projectPath ?? null,
            projectTemplateId: input.projectTemplateId ?? null,
            sourceChannelId: input.source?.channelId ?? null,
            sourceEventId: input.source?.eventId ?? null,
            sourceThreadRootId: input.source?.threadRootId ?? null,
            updatedAt: Math.floor(Date.now() / 1_000),
          },
        ],
      );
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current = []) => [
        ...current.filter((channel) => channel.id !== chat.id),
        chat,
      ]);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({
        queryKey: chatsQueryKey,
        refetchType: "none",
      });
      void queryClient.invalidateQueries({
        queryKey: chatMetadataListQueryKey,
        refetchType: "none",
      });
      void queryClient.invalidateQueries({
        queryKey: channelsQueryKey,
        refetchType: "none",
      });
    },
  });
}

export function useUpdateChatMetadataMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: UpdateChatMetadataInput) => updateChatMetadata(input),
    onSuccess: (metadata) => {
      queryClient.setQueryData(
        chatMetadataQueryKey(metadata.channelId),
        metadata,
      );
      queryClient.setQueryData<ChatMetadata[]>(
        chatMetadataListQueryKey,
        (current = []) => [
          ...current.filter((item) => item.channelId !== metadata.channelId),
          metadata,
        ],
      );
    },
  });
}

export function useSendChatContextMessageMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: SendChatContextMessageInput) =>
      sendChatContextMessage(input),
    onSuccess: (_result, input) => {
      void queryClient.invalidateQueries({
        queryKey: channelMessagesKey(input.channelId),
      });
    },
  });
}
