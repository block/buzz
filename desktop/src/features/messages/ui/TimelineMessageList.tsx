import * as React from "react";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import {
  buildTimelineItems,
  getTimelineItemKey,
  resolveActiveDayTimestamp,
  type TimelineItem,
  type TimelineItemsResult,
} from "@/features/messages/lib/timelineItems";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
  hasVideoAttachment,
} from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  type ListVirtualizer,
  VirtualizedList,
} from "@/shared/ui/VirtualizedList";
import { DayDivider } from "./DayDivider";
import { ActiveDayHeader } from "./ActiveDayHeader";
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
  isMessageUnreadById?: (messageId: string) => boolean;
  messageFooters?: Record<string, React.ReactNode>;
  /** Hoisted main-timeline entries (computed once in ChannelPane). Falls back
   *  to deriving them here when omitted (e.g. the deferred-render pass). */
  mainEntries?: MainTimelineEntry[];
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
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
  /** Caller-owned scroll container the virtualizer measures and scrolls. */
  scrollContainerRef: React.RefObject<HTMLElement | null>;
  /** Non-scrolling overlay container the floating active-day header portals
   *  into — sits OUTSIDE the scroll container so the header cannot drift. */
  headerOverlayRef: React.RefObject<HTMLElement | null>;
  /** Receives the flattened item stream + index map so the scroll manager can
   *  resolve scroll targets by id. Called whenever the stream is rebuilt. */
  onItems?: (result: TimelineItemsResult) => void;
  /** Receives the virtualizer instance for index-model scroll paths. */
  onVirtualizer?: (virtualizer: ListVirtualizer) => void;
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
  isMessageUnreadById,
  messageFooters,
  mainEntries,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
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
  scrollContainerRef,
  headerOverlayRef,
  onItems,
  onVirtualizer,
}: TimelineMessageListProps) {
  const entries = React.useMemo(
    () => mainEntries ?? buildMainTimelineEntries(messages),
    [mainEntries, messages],
  );
  const reviewCommentsByRootId = React.useMemo(
    () =>
      messages.some(hasVideoAttachment)
        ? buildVideoReviewCommentsByRootId(messages)
        : new Map<string, TimelineMessage[]>(),
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

  // The flattened item stream and its messageId -> itemIndex map are produced
  // together from ONE memo, keyed on the entries and the unread boundary (the
  // unread divider is its own item, so it shifts indices). A separate memo with
  // diverging deps would let the map go stale and scroll deep-links to the wrong
  // row — the exact failure virtualization risks.
  const itemsResult = React.useMemo(
    () => buildTimelineItems(entries, firstUnreadMessageId),
    [entries, firstUnreadMessageId],
  );

  React.useEffect(() => {
    onItems?.(itemsResult);
  }, [itemsResult, onItems]);

  const renderItem = React.useCallback(
    (item: TimelineItem) => {
      switch (item.kind) {
        case "day-divider":
          // Heading is resolved at render time (not baked into the item) so
          // "Today"/"Yesterday" track the wall clock, not build time.
          return <DayDivider label={formatDayHeading(item.headingTimestamp)} />;
        case "unread-divider":
          return <UnreadDivider />;
        case "system":
          return (
            <SystemRow
              currentPubkey={currentPubkey}
              entry={item.entry}
              footer={messageFooters?.[item.entry.message.id] ?? null}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
            />
          );
        case "message":
          return (
            <MessageRowItem
              agentPubkeys={agentPubkeys}
              channelId={channelId}
              currentPubkey={currentPubkey}
              entry={item.entry}
              followThreadById={followThreadById}
              footer={messageFooters?.[item.entry.message.id] ?? null}
              highlightedMessageId={highlightedMessageId}
              isFollowingThreadById={isFollowingThreadById}
              isUnread={isMessageUnreadById?.(item.entry.message.id)}
              onDelete={onDelete}
              onEdit={onEdit}
              onMarkRead={onMarkRead}
              onMarkUnread={onMarkUnread}
              onReply={onReply}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
              searchActiveMessageId={searchActiveMessageId}
              searchMatchingMessageIds={searchMatchingMessageIds}
              searchQuery={searchQuery}
              threadUnreadCounts={threadUnreadCounts}
              unfollowThreadById={unfollowThreadById}
              videoReviewContext={videoReviewContextById.get(
                item.entry.message.id,
              )}
            />
          );
      }
    },
    [
      agentPubkeys,
      channelId,
      currentPubkey,
      followThreadById,
      highlightedMessageId,
      isFollowingThreadById,
      isMessageUnreadById,
      messageFooters,
      onDelete,
      onEdit,
      onMarkRead,
      onMarkUnread,
      onReply,
      onToggleReaction,
      profiles,
      searchActiveMessageId,
      searchMatchingMessageIds,
      searchQuery,
      threadUnreadCounts,
      unfollowThreadById,
      videoReviewContextById,
    ],
  );

  // The floating active-day header is portaled by VirtualizedList into the
  // non-scrolling overlay container (outside the scroll element) and is
  // re-invoked with the topmost visible row index on every scroll. We resolve
  // that index back to the day it belongs to so the label tracks the viewport
  // even after that day's in-stream divider has been windowed out.
  const renderStickyHeader = React.useCallback(
    (topVisibleIndex: number | null) => {
      const headingTimestamp = resolveActiveDayTimestamp(
        itemsResult.items,
        topVisibleIndex,
      );
      if (headingTimestamp === null) {
        return null;
      }
      return <ActiveDayHeader label={formatDayHeading(headingTimestamp)} />;
    },
    [itemsResult.items],
  );

  return (
    <VirtualizedList
      getItemKey={getTimelineItemKey}
      headerOverlayRef={headerOverlayRef}
      innerClassName="flex flex-col"
      items={itemsResult.items}
      onVirtualizer={onVirtualizer}
      renderItem={renderItem}
      scrollRef={scrollContainerRef}
      stickyHeader={renderStickyHeader}
    />
  );
});

function SystemRow({
  currentPubkey,
  entry,
  footer,
  onToggleReaction,
  profiles,
}: {
  currentPubkey?: string;
  entry: MainTimelineEntry;
  footer: React.ReactNode;
  onToggleReaction?: TimelineMessageListProps["onToggleReaction"];
  profiles?: UserProfileLookup;
}) {
  return (
    <div className="flex flex-col gap-1 pb-2.5">
      <SystemMessageRow
        message={entry.message}
        currentPubkey={currentPubkey}
        onToggleReaction={onToggleReaction}
        profiles={profiles}
      />
      {footer}
    </div>
  );
}

type MessageRowItemProps = Pick<
  TimelineMessageListProps,
  | "agentPubkeys"
  | "channelId"
  | "currentPubkey"
  | "followThreadById"
  | "highlightedMessageId"
  | "isFollowingThreadById"
  | "onDelete"
  | "onEdit"
  | "onMarkUnread"
  | "onMarkRead"
  | "onReply"
  | "onToggleReaction"
  | "profiles"
  | "searchActiveMessageId"
  | "searchMatchingMessageIds"
  | "searchQuery"
  | "threadUnreadCounts"
  | "unfollowThreadById"
> & {
  entry: MainTimelineEntry;
  footer: React.ReactNode;
  isUnread?: boolean;
  videoReviewContext: ReturnType<typeof buildVideoReviewContextForMessage>;
};

function MessageRowItem({
  agentPubkeys,
  channelId,
  currentPubkey,
  entry,
  followThreadById,
  footer,
  highlightedMessageId,
  isFollowingThreadById,
  isUnread,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
  onReply,
  onToggleReaction,
  profiles,
  searchActiveMessageId,
  searchMatchingMessageIds,
  searchQuery,
  threadUnreadCounts,
  unfollowThreadById,
  videoReviewContext,
}: MessageRowItemProps) {
  const { message, summary } = entry;
  const canDelete =
    onDelete && currentPubkey && message.pubkey === currentPubkey
      ? onDelete
      : undefined;
  const canEdit =
    onEdit && currentPubkey && message.pubkey === currentPubkey
      ? onEdit
      : undefined;

  if (summary && onReply) {
    const isHighlighted = message.id === highlightedMessageId;
    return (
      <div
        className={cn(
          "group/message relative mx-1 flex flex-col gap-0 rounded-2xl px-0 py-1 pb-2.5 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
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
          isUnread={isUnread}
          message={message}
          onDelete={canDelete}
          onEdit={canEdit}
          onFollowThread={
            followThreadById ? () => followThreadById(message.id) : undefined
          }
          onMarkRead={onMarkRead}
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
          videoReviewContext={videoReviewContext}
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
      </div>
    );
  }

  const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
  const isSearchActive = message.id === searchActiveMessageId;

  return (
    <div className="flex flex-col gap-1 pb-2.5">
      <MessageRow
        agentPubkeys={agentPubkeys}
        channelId={channelId}
        highlighted={message.id === highlightedMessageId || isSearchActive}
        isUnread={isUnread}
        message={message}
        onDelete={canDelete}
        onEdit={canEdit}
        onMarkRead={onMarkRead}
        onMarkUnread={onMarkUnread}
        onToggleReaction={onToggleReaction}
        onReply={onReply}
        profiles={profiles}
        searchQuery={isSearchMatch ? searchQuery : undefined}
        showDepthGuides={false}
        videoReviewContext={videoReviewContext}
      />
      {footer}
    </div>
  );
}
