import { getThreadReference } from "@/features/messages/lib/threading";
import { getEventById } from "@/shared/api/tauri";
import type { SearchHit } from "@/shared/api/types";
import { KIND_FORUM_COMMENT, KIND_FORUM_POST } from "@/shared/constants/kinds";

export type SearchHitDestination =
  | {
      kind: "chat";
      chatId: string;
      messageId?: string;
    }
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

export async function resolveSearchHitDestination(
  hit: SearchHit,
): Promise<SearchHitDestination | null> {
  if (!hit.channelId) {
    return null;
  }

  if (hit.channelType === "chat") {
    return {
      kind: "chat",
      chatId: hit.channelId,
      messageId: hit.eventId,
    };
  }

  if (hit.kind === KIND_FORUM_POST) {
    return {
      kind: "forum-post",
      channelId: hit.channelId,
      postId: hit.eventId,
    };
  }

  if (hit.kind === KIND_FORUM_COMMENT) {
    try {
      const event = await getEventById(hit.eventId);
      const thread = getThreadReference(event.tags);
      const postId = thread.rootId ?? thread.parentId ?? null;

      if (!postId) {
        return {
          kind: "channel",
          channelId: hit.channelId,
        };
      }

      return {
        kind: "forum-post",
        channelId: hit.channelId,
        postId,
        replyId: hit.eventId,
      };
    } catch (error) {
      console.error(
        "Failed to resolve forum reply search destination",
        hit.eventId,
        error,
      );

      return {
        kind: "channel",
        channelId: hit.channelId,
      };
    }
  }

  return {
    kind: "channel",
    channelId: hit.channelId,
    messageId: hit.eventId,
    threadRootId: hit.threadRootId ?? null,
  };
}
