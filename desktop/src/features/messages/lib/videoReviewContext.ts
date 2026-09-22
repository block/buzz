import { fromMarkdown } from "mdast-util-from-markdown";

import type { TimelineMessage } from "@/features/messages/types";
import { isVoiceNoteAttachment } from "@/features/messages/lib/audioAttachment";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { isVideoMedia } from "@/shared/ui/markdown/mediaEntry";
import {
  parseImetaTags,
  type ParsedImetaEntry,
} from "@/shared/ui/markdown/parseImeta";
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

type VideoRootPredicate = (
  message: Pick<TimelineMessage, "body" | "tags">,
) => boolean;

type MarkdownAstNode = {
  children?: MarkdownAstNode[];
  identifier?: string;
  type: string;
  url?: string;
};

function isReviewVideo(
  url: string,
  entry: ParsedImetaEntry | undefined,
): boolean {
  return isVideoMedia(url, entry?.m) && !isVoiceNoteAttachment(entry);
}

function markdownImageUrls(body: string): string[] {
  if (!body.includes("![")) return [];

  const definitions = new Map<string, string>();
  const directUrls: string[] = [];
  const referenceIds: string[] = [];

  const visit = (node: MarkdownAstNode) => {
    if (node.type === "definition" && node.identifier && node.url) {
      if (!definitions.has(node.identifier)) {
        definitions.set(node.identifier, node.url);
      }
    } else if (node.type === "image" && node.url) {
      directUrls.push(node.url);
    } else if (node.type === "imageReference" && node.identifier) {
      referenceIds.push(node.identifier);
    }

    node.children?.forEach(visit);
  };

  visit(fromMarkdown(body) as MarkdownAstNode);
  return [
    ...directUrls,
    ...referenceIds.flatMap((identifier) => {
      const url = definitions.get(identifier);
      return url ? [url] : [];
    }),
  ];
}

/**
 * Returns whether a message contains a video URL in a Markdown image that
 * the renderer will actually mount. Orphan imeta entries are intentionally
 * excluded because they do not produce a video player.
 */
export function hasRenderedVideoAttachment(
  message: Pick<TimelineMessage, "body" | "tags">,
): boolean {
  const imetaByUrl = parseImetaTags(message.tags ?? []);
  return markdownImageUrls(message.body).some((src) =>
    isReviewVideo(src, imetaByUrl.get(src)),
  );
}

export function hasVideoAttachment(
  message: Pick<TimelineMessage, "body" | "tags">,
): boolean {
  const imetaByUrl = parseImetaTags(message.tags ?? []);
  if (
    [...imetaByUrl.values()].some((entry) => isReviewVideo(entry.url, entry))
  ) {
    return true;
  }

  for (const src of markdownImageUrls(message.body)) {
    if (isReviewVideo(src, imetaByUrl.get(src))) return true;
  }

  return false;
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

export function buildVideoReviewCommentRootIdsByMessageId(
  messages: TimelineMessage[],
  videoRootPredicate: VideoRootPredicate = hasVideoAttachment,
): ReadonlyMap<string, string> {
  const messageById = new Map(messages.map((message) => [message.id, message]));
  const videoMessageIds = new Set(
    messages.filter(videoRootPredicate).map((message) => message.id),
  );
  const rootIdsByMessageId = new Map<string, string>();

  for (const message of messages) {
    if (videoMessageIds.has(message.id)) continue;

    let ancestorId = message.parentId ?? null;
    const visited = new Set<string>();
    while (ancestorId && !visited.has(ancestorId)) {
      if (videoMessageIds.has(ancestorId)) {
        rootIdsByMessageId.set(message.id, ancestorId);
        break;
      }
      visited.add(ancestorId);
      ancestorId = messageById.get(ancestorId)?.parentId ?? null;
    }
  }

  return rootIdsByMessageId;
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
  videoRootPredicate = hasVideoAttachment,
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
  videoRootPredicate?: VideoRootPredicate;
}): VideoReviewContext | undefined {
  if (!videoRootPredicate(message)) {
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

export function buildVideoReviewContextsByMessageId({
  channelId,
  channelName,
  channelType,
  isSendingVideoReviewComment = false,
  messages,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  videoRootPredicate = hasVideoAttachment,
}: {
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  isSendingVideoReviewComment?: boolean;
  messages: TimelineMessage[];
  onSendVideoReviewComment?: SendVideoReviewComment;
  onToggleReaction?: ToggleMessageReaction;
  profiles?: UserProfileLookup;
  videoRootPredicate?: VideoRootPredicate;
}): ReadonlyMap<string, VideoReviewContext> {
  const contexts = new Map<string, VideoReviewContext>();
  if (!messages.some(videoRootPredicate)) {
    return contexts;
  }

  const commentsByRootId = buildVideoReviewCommentsByRootId(messages);
  for (const message of messages) {
    const context = buildVideoReviewContextForMessage({
      channelId,
      channelName,
      channelType,
      comments: commentsByRootId.get(message.id) ?? [],
      isSendingVideoReviewComment,
      message,
      onSendVideoReviewComment,
      onToggleReaction,
      profiles,
      videoRootPredicate,
    });
    if (context) {
      contexts.set(message.id, context);
    }
  }

  return contexts;
}

/**
 * Builds the paired video-review maps used by timeline presentation: contexts
 * are keyed by video message, while comment roots map each descendant back to
 * its nearest video ancestor.
 */
export function buildVideoReviewPresentationByMessageId(
  args: Parameters<typeof buildVideoReviewContextsByMessageId>[0],
  videoRootPredicate: VideoRootPredicate = hasVideoAttachment,
) {
  return {
    commentRootIdsByMessageId: buildVideoReviewCommentRootIdsByMessageId(
      args.messages,
      videoRootPredicate,
    ),
    contextsByMessageId: buildVideoReviewContextsByMessageId({
      ...args,
      videoRootPredicate,
    }),
  };
}

/** The synchronized context and comment-root maps for a rendered timeline. */
export type VideoReviewPresentation = ReturnType<
  typeof buildVideoReviewPresentationByMessageId
>;
