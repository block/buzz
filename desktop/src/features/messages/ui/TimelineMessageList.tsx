import * as React from "react";

import {
  formatDayHeading,
  isSameDay,
} from "@/features/messages/lib/dateFormatters";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";

type TimelineMessageListProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  currentPubkey?: string;
  followThreadById?: (rootId: string) => void;
  highlightedMessageId?: string | null;
  isFollowingThreadById?: (rootId: string) => boolean;
  messageFooters?: Record<string, React.ReactNode>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  isSendingVideoReviewComment?: boolean;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
};

function hasVideoAttachment(message: TimelineMessage): boolean {
  if (message.body.includes("![video](")) return true;

  return (
    message.tags?.some(
      (tag) =>
        tag[0] === "imeta" &&
        tag.some((part) => part.toLowerCase().startsWith("m video/")),
    ) ?? false
  );
}

function buildReviewCommentsByRootId(
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

export const TimelineMessageList = React.memo(function TimelineMessageList({
  agentPubkeys,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  followThreadById,
  highlightedMessageId = null,
  isFollowingThreadById,
  messageFooters,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  personaLookup,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  unfollowThreadById,
}: TimelineMessageListProps) {
  const entries = React.useMemo(
    () => buildMainTimelineEntries(messages),
    [messages],
  );
  const reviewCommentsByRootId = React.useMemo(
    () => buildReviewCommentsByRootId(messages),
    [messages],
  );
  // Contexts are memoized per message id so MessageRow/Markdown memo
  // comparisons hold across unrelated timeline re-renders (typing
  // indicators, presence updates) — a fresh context object per render would
  // defeat the memo and re-render every video message on every pass.
  const videoReviewContextById = React.useMemo(() => {
    const contexts = new Map<string, VideoReviewContext>();
    for (const message of messages) {
      if (!hasVideoAttachment(message)) continue;
      const comments = reviewCommentsByRootId.get(message.id) ?? [];
      contexts.set(message.id, {
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
      });
    }
    return contexts;
  }, [
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messages,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
    reviewCommentsByRootId,
  ]);
  const dayGroups: Array<{
    key: string;
    label: string;
    elements: React.ReactNode[];
  }> = [];
  let currentDayGroup: (typeof dayGroups)[number] | null = null;

  for (let i = 0; i < entries.length; i++) {
    const { message, summary } = entries[i];
    const prev = i > 0 ? entries[i - 1]?.message : null;
    const messageRenderKey = message.renderKey ?? message.id;

    if (!prev || !isSameDay(prev.createdAt, message.createdAt)) {
      currentDayGroup = {
        key: `day-${message.createdAt}`,
        label: formatDayHeading(message.createdAt),
        elements: [],
      };
      dayGroups.push(currentDayGroup);
    }

    if (message.kind === KIND_SYSTEM_MESSAGE) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup?.elements.push(
        <div key={messageRenderKey} className="flex flex-col gap-1">
          <SystemMessageRow
            message={message}
            agentPubkeys={agentPubkeys}
            currentPubkey={currentPubkey}
            onToggleReaction={onToggleReaction}
            personaLookup={personaLookup}
            profiles={profiles}
          />
          {footer}
        </div>,
      );
    } else if (summary && onReply) {
      const footer = messageFooters?.[message.id] ?? null;
      const isHighlighted = message.id === highlightedMessageId;
      currentDayGroup?.elements.push(
        <div
          key={messageRenderKey}
          className={cn(
            "group/message relative -mx-1 flex flex-col gap-0 rounded-2xl px-1 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
            isHighlighted &&
              "-mx-4 px-4 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-6",
          )}
        >
          <MessageRow
            agentPubkeys={agentPubkeys}
            channelId={channelId}
            highlighted={false}
            hoverBackground={false}
            isFollowingThread={
              isFollowingThreadById
                ? isFollowingThreadById(message.id)
                : undefined
            }
            message={message}
            onDelete={
              onDelete && currentPubkey && message.pubkey === currentPubkey
                ? onDelete
                : undefined
            }
            onEdit={
              onEdit && currentPubkey && message.pubkey === currentPubkey
                ? onEdit
                : undefined
            }
            onFollowThread={
              followThreadById ? () => followThreadById(message.id) : undefined
            }
            onMarkUnread={onMarkUnread}
            onToggleReaction={onToggleReaction}
            onReply={onReply}
            onUnfollowThread={
              unfollowThreadById
                ? () => unfollowThreadById(message.id)
                : undefined
            }
            profiles={profiles}
            videoReviewContext={videoReviewContextById.get(message.id)}
          />
          <MessageThreadSummaryRow
            depth={message.depth}
            message={message}
            onOpenThread={onReply}
            summary={summary}
          />
          {footer}
        </div>,
      );
    } else {
      const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
      const isSearchActive = message.id === searchActiveMessageId;
      const footer = messageFooters?.[message.id] ?? null;

      currentDayGroup?.elements.push(
        <div key={messageRenderKey} className="flex flex-col gap-1">
          <MessageRow
            agentPubkeys={agentPubkeys}
            channelId={channelId}
            highlighted={message.id === highlightedMessageId || isSearchActive}
            message={message}
            onDelete={
              onDelete && currentPubkey && message.pubkey === currentPubkey
                ? onDelete
                : undefined
            }
            onEdit={
              onEdit && currentPubkey && message.pubkey === currentPubkey
                ? onEdit
                : undefined
            }
            onMarkUnread={onMarkUnread}
            onToggleReaction={onToggleReaction}
            onReply={onReply}
            profiles={profiles}
            searchQuery={isSearchMatch ? searchQuery : undefined}
            videoReviewContext={videoReviewContextById.get(message.id)}
          />
          {footer}
        </div>,
      );
    }
  }

  return dayGroups.map((group) => (
    <section
      className="relative flex flex-col gap-2.5 before:absolute before:inset-x-0 before:top-[15px] before:h-px before:bg-border/35 before:content-['']"
      key={group.key}
    >
      <DayDivider label={group.label} />
      {group.elements}
    </section>
  ));
});
