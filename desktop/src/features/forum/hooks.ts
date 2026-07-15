import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getForumPosts, getForumThread } from "@/shared/api/forum";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type {
  Channel,
  ForumPostsResponse,
  ForumThreadResponse,
} from "@/shared/api/types";
import { KIND_FORUM_COMMENT, KIND_FORUM_POST } from "@/shared/constants/kinds";

export function forumPostsQueryKey(channelId: string) {
  return ["forum-posts", channelId] as const;
}

export function forumThreadQueryKey(channelId: string, eventId: string) {
  return ["forum-thread", channelId, eventId] as const;
}

export function useForumPostsQuery(channel: Channel | null) {
  const channelId = channel?.id ?? "";
  const enabled = channel !== null && channel.channelType === "forum";
  const relaySelfPubkey = useRelaySelfQuery(enabled).data;

  return useQuery<ForumPostsResponse>({
    enabled,
    queryKey: [...forumPostsQueryKey(channelId), relaySelfPubkey ?? null],
    queryFn: () => getForumPosts(channelId, 50, undefined, relaySelfPubkey),
    staleTime: 15_000,
    refetchInterval: 15_000,
  });
}

export function useForumThreadQuery(
  channelId: string | null,
  eventId: string | null,
) {
  const enabled = channelId !== null && eventId !== null;
  const relaySelfPubkey = useRelaySelfQuery(enabled).data;

  return useQuery<ForumThreadResponse>({
    enabled,
    queryKey: [
      ...forumThreadQueryKey(channelId ?? "", eventId ?? ""),
      relaySelfPubkey ?? null,
    ],
    queryFn: () =>
      getForumThread(
        channelId ?? "",
        eventId ?? "",
        undefined,
        undefined,
        relaySelfPubkey,
      ),
    staleTime: 10_000,
    refetchInterval: 10_000,
  });
}

export function useCreateForumPostMutation(channel: Channel | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      content,
      mentionPubkeys,
      mediaTags,
    }: {
      content: string;
      mentionPubkeys?: string[];
      mediaTags?: string[][];
    }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }

      return sendChannelMessage(
        channel.id,
        content,
        null,
        mediaTags,
        mentionPubkeys,
        KIND_FORUM_POST,
      );
    },
    onSuccess: () => {
      if (channel) {
        void queryClient.invalidateQueries({
          queryKey: forumPostsQueryKey(channel.id),
        });
      }
    },
  });
}

export function useDeleteForumPostMutation(channel: Channel | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ eventId }: { eventId: string }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }
      await deleteMessage(channel.id, eventId);
    },
    onSuccess: () => {
      if (channel) {
        void queryClient.invalidateQueries({
          queryKey: forumPostsQueryKey(channel.id),
        });
      }
    },
  });
}

export function useDeleteForumReplyMutation(
  channel: Channel | null,
  rootEventId: string | null,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ eventId }: { eventId: string }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }
      await deleteMessage(channel.id, eventId);
    },
    onSuccess: () => {
      if (channel) {
        if (rootEventId) {
          void queryClient.invalidateQueries({
            queryKey: forumThreadQueryKey(channel.id, rootEventId),
          });
        }
        void queryClient.invalidateQueries({
          queryKey: forumPostsQueryKey(channel.id),
        });
      }
    },
  });
}

export function useCreateForumReplyMutation(channel: Channel | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      content,
      parentEventId,
      mentionPubkeys,
      mediaTags,
    }: {
      content: string;
      parentEventId: string;
      mentionPubkeys?: string[];
      mediaTags?: string[][];
    }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }

      return sendChannelMessage(
        channel.id,
        content,
        parentEventId,
        mediaTags,
        mentionPubkeys,
        KIND_FORUM_COMMENT,
      );
    },
    onSuccess: (_data, variables) => {
      if (channel) {
        void queryClient.invalidateQueries({
          queryKey: forumThreadQueryKey(channel.id, variables.parentEventId),
        });
        void queryClient.invalidateQueries({
          queryKey: forumPostsQueryKey(channel.id),
        });
      }
    },
  });
}
