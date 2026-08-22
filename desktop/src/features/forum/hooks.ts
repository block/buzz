import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getForumPosts, getForumThread } from "@/shared/api/forum";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useProfileQuery } from "@/features/profile/hooks";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type {
  Channel,
  ForumPostsResponse,
  ForumThreadResponse,
} from "@/shared/api/types";
import { KIND_FORUM_COMMENT, KIND_FORUM_POST } from "@/shared/constants/kinds";
import { replyRecipientPubkeys } from "@/features/messages/lib/threading";

/** Keeps focused polling for forum posts at the established 15-second cadence. */
export const FORUM_POSTS_REFETCH_INTERVAL_MS = 15_000;
/** Keeps focused polling for forum threads at the established 10-second cadence. */
export const FORUM_THREAD_REFETCH_INTERVAL_MS = 10_000;
/** Suppresses the focus refetch until forum data is genuinely stale.
 * Both families poll while focused and have push-invalidation from mutations. */
export const FORUM_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy shared by forum-posts and forum-thread queries; consumed by focusRefetchPolicy.test.mjs. */
export const forumFocusRefetchPolicy = {
  staleTime: FORUM_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: false,
} as const;

export function forumPostsQueryKey(channelId: string) {
  return ["forum-posts", channelId] as const;
}

export function forumThreadQueryKey(channelId: string, eventId: string) {
  return ["forum-thread", channelId, eventId] as const;
}

export function useForumPostsQuery(channel: Channel | null) {
  const refetchInterval = useFocusedRefetchInterval(
    FORUM_POSTS_REFETCH_INTERVAL_MS,
  );

  const channelId = channel?.id ?? "";
  const enabled = channel !== null && channel.channelType === "forum";
  const relaySelfPubkey = useRelaySelfQuery(enabled).data;

  return useQuery<ForumPostsResponse>({
    enabled,
    queryKey: [...forumPostsQueryKey(channelId), relaySelfPubkey ?? null],
    queryFn: () => getForumPosts(channelId, 50, undefined, relaySelfPubkey),
    refetchInterval,
    ...forumFocusRefetchPolicy,
  });
}

export function useForumThreadQuery(
  channelId: string | null,
  eventId: string | null,
) {
  const refetchInterval = useFocusedRefetchInterval(
    FORUM_THREAD_REFETCH_INTERVAL_MS,
  );

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
    refetchInterval,
    ...forumFocusRefetchPolicy,
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

export function useCreateForumReplyMutation(
  channel: Channel | null,
  currentPubkey?: string | null,
) {
  const queryClient = useQueryClient();
  const profileQuery = useProfileQuery();

  return useMutation({
    mutationFn: async ({
      content,
      parentEventId,
      parentAuthorPubkey,
      mentionPubkeys,
      mediaTags,
    }: {
      content: string;
      parentEventId: string;
      /**
       * Author of the post being answered. Forum channels are mention-eligible
       * for agents, and an agent's `require_mention` subscription is a `#p`
       * REQ filter — an untagged comment never reaches it.
       */
      parentAuthorPubkey?: string | null;
      mentionPubkeys?: string[];
      mediaTags?: string[][];
    }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }

      // Self must be dropped here, not by the caller: `ForumView` compares
      // against a `currentPubkey` that is `undefined` until identity resolves,
      // so replying to your own post early would self-`p`-tag. Falling back to
      // the profile query matters for the same reason — passing `""` would
      // seed the dedupe set with the empty string and drop nobody.
      // If identity is somehow still unknown, keep the tag: losing it costs
      // the agent the comment entirely, which is worse than a stray self-tag.
      const selfPubkey = currentPubkey ?? profileQuery.data?.pubkey ?? "";
      const recipientPubkeys = parentAuthorPubkey
        ? replyRecipientPubkeys({
            currentPubkey: selfPubkey,
            mentionPubkeys: mentionPubkeys ?? [],
            parentAuthorPubkey,
          })
        : mentionPubkeys;

      return sendChannelMessage(
        channel.id,
        content,
        parentEventId,
        mediaTags,
        recipientPubkeys,
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
