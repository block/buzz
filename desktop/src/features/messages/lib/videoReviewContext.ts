import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";

type SendVideoReviewComment = (
  message: TimelineMessage,
  content: string,
  mentionPubkeys: string[],
  mediaTags?: string[][],
  parentEventId?: string,
) => Promise<void>;

type ToggleMessageReaction = (
  message: TimelineMessage,
  emoji: string,
  remove: boolean,
) => Promise<void>;

export function hasVideoAttachment(message: TimelineMessage): boolean {
  if (message.body.includes("![video](")) return true;

  return (
    message.tags?.some(
      (tag) =>
        tag[0] === "imeta" &&
        tag.some((part) => part.toLowerCase().startsWith("m video/")),
    ) ?? false
  );
}

export function buildVideoReviewCommentsByRootId(
  messages: TimelineMessage[],
): Map<string, TimelineMessage[]> {
  const messageById = new Map(messages.map((message) => [message.id, message]));
  const commentsByRootId = new Map<string, TimelineMessage[]>();

  for (const message of messages) {
    let ancestorId = message.parentId ?? null;
    let hops = 0;
    const maxHops = messages.length + 1;

    while (ancestorId && hops < maxHops) {
      const comments = commentsByRootId.get(ancestorId) ?? [];
      comments.push(message);
      commentsByRootId.set(ancestorId, comments);
      ancestorId = messageById.get(ancestorId)?.parentId ?? null;
      hops += 1;
    }
  }

  for (const comments of commentsByRootId.values()) {
    comments.sort((left, right) => {
      if (left.createdAt !== right.createdAt) {
        return left.createdAt - right.createdAt;
      }
      return left.id.localeCompare(right.id);
    });
  }

  return commentsByRootId;
}

export function buildVideoReviewCommentsForRoot(
  messages: TimelineMessage[],
  rootId: string,
): TimelineMessage[] {
  const messageById = new Map(messages.map((message) => [message.id, message]));
  const comments: TimelineMessage[] = [];

  for (const message of messages) {
    let ancestorId = message.parentId ?? null;
    let hops = 0;
    const maxHops = messages.length + 1;

    while (ancestorId && hops < maxHops) {
      if (ancestorId === rootId) {
        comments.push(message);
        break;
      }
      ancestorId = messageById.get(ancestorId)?.parentId ?? null;
      hops += 1;
    }
  }

  comments.sort((left, right) => {
    if (left.createdAt !== right.createdAt) {
      return left.createdAt - right.createdAt;
    }
    return left.id.localeCompare(right.id);
  });

  return comments;
}

export function buildVideoReviewContextForMessage({
  channelId,
  channelName,
  channelType,
  comments,
  isSendingVideoReviewComment = false,
  message,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
}: {
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  comments: TimelineMessage[];
  isSendingVideoReviewComment?: boolean;
  message: TimelineMessage;
  onSendVideoReviewComment?: SendVideoReviewComment;
  onToggleReaction?: ToggleMessageReaction;
  profiles?: UserProfileLookup;
}): VideoReviewContext | undefined {
  if (!hasVideoAttachment(message)) {
    return undefined;
  }

  return {
    channelId,
    channelName,
    channelType,
    comments,
    disabled: !onSendVideoReviewComment || message.pending,
    isSending: isSendingVideoReviewComment,
    onSendComment: onSendVideoReviewComment
      ? (content, mentionPubkeys, mediaTags, parentEventId) =>
          onSendVideoReviewComment(
            message,
            content,
            mentionPubkeys,
            mediaTags,
            parentEventId,
          )
      : undefined,
    onToggleCommentReaction: onToggleReaction
      ? (comment, emoji, remove) => {
          const sourceComment = comments.find(
            (candidate) => candidate.id === comment.id,
          );
          if (!sourceComment) return Promise.resolve();
          return onToggleReaction(sourceComment, emoji, remove);
        }
      : undefined,
    profiles,
    rootEventId: message.id,
  };
}
