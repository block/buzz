import * as React from "react";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import {
  buildMainTimelineEntries,
  shouldRenderUnreadDivider,
} from "@/features/messages/lib/threadPanel";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
} from "@/features/messages/lib/videoReviewContext";
import { buildDayGroupBoundaries } from "@/features/messages/lib/timelineSnapshot";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";
import { UnreadDivider } from "./UnreadDivider";

type TimelineMessageListProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  currentPubkey?: string;
  /** Event id of the oldest unread top-level message; renders a "New" divider above it. */
  firstUnreadMessageId?: string | null;
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
  /** Per-thread unread counts keyed by thread root id. */
  threadUnreadCounts?: ReadonlyMap<string, number>;
};

export const TimelineMessageList = React.memo(function TimelineMessageList({
  agentPubkeys,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  firstUnreadMessageId = null,
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
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  threadUnreadCounts,
  unfollowThreadById,
}: TimelineMessageListProps) {
  const entries = React.useMemo(
    () => buildMainTimelineEntries(messages),
    [messages],
  );
  const reviewCommentsByRootId = React.useMemo(
    () => buildVideoReviewCommentsByRootId(messages),
    [messages],
  );
  // Contexts are memoized per message id so MessageRow/Markdown memo
  // comparisons hold across unrelated timeline re-renders (typing
  // indicators, presence updates) — a fresh context object per render would
  // defeat the memo and re-render every video message on every pass.
  const videoReviewContextById = React.useMemo(() => {
    const contexts = new Map<
      string,
      NonNullable<ReturnType<typeof buildVideoReviewContextForMessage>>
    >();
    for (const message of messages) {
      const comments = reviewCommentsByRootId.get(message.id) ?? [];
      const context = buildVideoReviewContextForMessage({
        channelId,
        channelName,
        channelType,
        comments,
        isSendingVideoReviewComment,
        message,
        onSendVideoReviewComment,
        onToggleReaction,
        profiles,
      });
      if (context) {
        contexts.set(message.id, context);
      }
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

  // Day-divider decision delegated to a pure, lib-tested helper: a new group
  // starts at index 0 and whenever a message falls on a different calendar day
  // than the one before it. We index the boundary start positions so the render
  // loop below stays a straight walk while the grouping logic lives in `lib/`.
  const dayGroupStartIndices = new Set(
    buildDayGroupBoundaries(entries.map((entry) => entry.message)).map(
      (boundary) => boundary.startIndex,
    ),
  );

  for (let i = 0; i < entries.length; i++) {
    const { message, summary } = entries[i];
    const messageRenderKey = message.renderKey ?? message.id;

    if (dayGroupStartIndices.has(i)) {
      currentDayGroup = {
        key: `day-${message.createdAt}`,
        label: formatDayHeading(message.createdAt),
        elements: [],
      };
      dayGroups.push(currentDayGroup);
    }

    // The unread "New" divider only marks a read/unread boundary when there is
    // a message above the first unread. When the first unread is the first
    // rendered top-level entry (fresh/never-read channel), there is nothing
    // above to separate from, so it is suppressed.
    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      currentDayGroup?.elements.push(
        <UnreadDivider key={`unread-${messageRenderKey}`} />,
      );
    }

    if (message.kind === KIND_SYSTEM_MESSAGE) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup?.elements.push(
        <div key={messageRenderKey} className="flex flex-col gap-1">
          <SystemMessageRow
            message={message}
            currentPubkey={currentPubkey}
            onToggleReaction={onToggleReaction}
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
            "group/message relative mx-1 flex flex-col gap-0 rounded-2xl px-0 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
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
            showDepthGuides={false}
            videoReviewContext={videoReviewContextById.get(message.id)}
          />
          <MessageThreadSummaryRow
            depth={message.depth}
            message={message}
            onOpenThread={onReply}
            showDepthGuides={false}
            summary={summary}
            unreadCount={threadUnreadCounts?.get(message.id)}
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
            showDepthGuides={false}
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
