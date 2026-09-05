import { getThreadReference } from "@/features/messages/lib/threading";
import { getEventById } from "@/shared/api/tauri";
import { KIND_FORUM_COMMENT, KIND_FORUM_POST } from "@/shared/constants/kinds";

/**
 * Where a `buzz://message` link should land.
 *
 * Mirrors `SearchHitDestination`. Search already resolves this correctly —
 * `openSearchHitWithNavigation` branches to `goForumPost` for forum targets —
 * but message links (deep links and the in-app markdown handler) routed
 * everything through `goChannel`. `/channels/$channelId` hardcodes
 * `selectedPostId={null}`, and `useChannelRouteTarget` returns early for
 * `channelType === "forum"`, so a forum target landed on the post list with
 * nothing selected.
 */
export type MessageLinkDestination =
  | {
      kind: "channel";
      channelId: string;
      messageId?: string;
      threadRootId?: string | null;
    }
  | {
      kind: "forum-post";
      channelId: string;
      postId: string;
      replyId?: string;
    };

/**
 * Resolves a message link by fetching the target event and branching on its
 * kind, the way `resolveSearchHitDestination` does for search hits.
 *
 * A message link carries no kind of its own, so the event has to be fetched.
 * Every failure path falls back to the channel destination the caller would
 * have used anyway, so a link never becomes unclickable because the lookup
 * failed.
 */
export async function resolveMessageLinkDestination(
  channelId: string,
  messageId: string,
  threadRootId?: string | null,
  fetchEvent: typeof getEventById = getEventById,
): Promise<MessageLinkDestination> {
  const channelDestination: MessageLinkDestination = {
    kind: "channel",
    channelId,
    messageId,
    threadRootId: threadRootId ?? null,
  };

  try {
    const event = await fetchEvent(messageId);

    if (event.kind === KIND_FORUM_POST) {
      return { kind: "forum-post", channelId, postId: messageId };
    }

    if (event.kind === KIND_FORUM_COMMENT) {
      const thread = getThreadReference(event.tags);
      const postId = thread.rootId ?? thread.parentId ?? null;

      if (!postId) {
        return channelDestination;
      }

      return {
        kind: "forum-post",
        channelId,
        postId,
        replyId: messageId,
      };
    }

    return channelDestination;
  } catch (error) {
    console.error(
      "Failed to resolve message link destination",
      messageId,
      error,
    );
    return channelDestination;
  }
}
