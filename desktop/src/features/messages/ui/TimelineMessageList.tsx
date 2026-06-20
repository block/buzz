import * as React from "react";

import {
  formatDayHeading,
  isSameDay,
  startOfLocalDaySeconds,
} from "@/features/messages/lib/dateFormatters";
import {
  buildMainTimelineEntries,
  shouldRenderUnreadDivider,
} from "@/features/messages/lib/threadPanel";
import {
  buildVideoReviewCommentsForRoot,
  buildVideoReviewContextForMessage,
  hasVideoAttachment,
} from "@/features/messages/lib/videoReviewContext";
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

type TimelineDayRow = {
  key: string;
  label: string;
  type: "day";
};

type TimelineUnreadRow = {
  key: string;
  type: "unread";
};

type TimelineMessageRowModel = {
  key: string;
  message: TimelineMessage;
  summary: ReturnType<typeof buildMainTimelineEntries>[number]["summary"];
  type: "message";
};

type TimelineRenderRow =
  | TimelineDayRow
  | TimelineUnreadRow
  | TimelineMessageRowModel;

type TimelineNonDayRow = TimelineUnreadRow | TimelineMessageRowModel;

type TimelineDayGroup = {
  key: string;
  label: string;
  rows: TimelineNonDayRow[];
};

function buildTimelineRenderRows({
  firstUnreadMessageId,
  messages,
}: {
  firstUnreadMessageId: string | null;
  messages: TimelineMessage[];
}): TimelineRenderRow[] {
  const entries = buildMainTimelineEntries(messages);
  const rows: TimelineRenderRow[] = [];
  let previousMessage: TimelineMessage | null = null;

  for (let i = 0; i < entries.length; i++) {
    const { message, summary } = entries[i];
    const messageRenderKey = message.renderKey ?? message.id;

    if (
      !previousMessage ||
      !isSameDay(previousMessage.createdAt, message.createdAt)
    ) {
      rows.push({
        key: `day-${startOfLocalDaySeconds(message.createdAt)}`,
        label: formatDayHeading(message.createdAt),
        type: "day",
      });
    }

    // The unread "New" divider only marks a read/unread boundary when there is
    // a message above the first unread. When the first unread is the first
    // rendered top-level entry (fresh/never-read channel), there is nothing
    // above to separate from, so it is suppressed.
    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      rows.push({ key: `unread:${messageRenderKey}`, type: "unread" });
    }

    rows.push({
      key: `msg:${messageRenderKey}`,
      message,
      summary,
      type: "message",
    });

    previousMessage = message;
  }

  return rows;
}

function buildTimelineDayGroups(rows: TimelineRenderRow[]): TimelineDayGroup[] {
  const groups: TimelineDayGroup[] = [];
  let currentGroup: TimelineDayGroup | null = null;

  for (const row of rows) {
    if (row.type === "day") {
      currentGroup = {
        key: row.key,
        label: row.label,
        rows: [],
      };
      groups.push(currentGroup);
      continue;
    }

    if (!currentGroup) {
      currentGroup = {
        key: "day-undated",
        label: "",
        rows: [],
      };
      groups.push(currentGroup);
    }

    currentGroup.rows.push(row);
  }

  return groups;
}

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
  const rows = React.useMemo(
    () => buildTimelineRenderRows({ firstUnreadMessageId, messages }),
    [firstUnreadMessageId, messages],
  );
  const dayGroups = React.useMemo(() => buildTimelineDayGroups(rows), [rows]);

  return (
    <div className="flex flex-col gap-0">
      {dayGroups.map((group) => (
        <section
          className="relative flex flex-col gap-2 before:absolute before:inset-x-0 before:top-4 before:h-px before:bg-border/35 before:content-['']"
          data-day-label={group.label}
          data-testid="message-timeline-day-group"
          key={group.key}
        >
          {group.label ? <DayDivider label={group.label} /> : null}
          {group.rows.map((row) => (
            <TimelineRenderRowView
              agentPubkeys={agentPubkeys}
              allMessages={
                row.type === "message" && hasVideoAttachment(row.message)
                  ? messages
                  : undefined
              }
              channelId={channelId}
              channelName={channelName}
              channelType={channelType}
              currentPubkey={currentPubkey}
              followThreadById={followThreadById}
              highlightedMessageId={highlightedMessageId}
              isFollowingThreadById={isFollowingThreadById}
              isSendingVideoReviewComment={isSendingVideoReviewComment}
              key={row.key}
              messageFooters={messageFooters}
              onDelete={onDelete}
              onEdit={onEdit}
              onMarkUnread={onMarkUnread}
              onReply={onReply}
              onSendVideoReviewComment={onSendVideoReviewComment}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
              row={row}
              searchActiveMessageId={searchActiveMessageId}
              searchMatchingMessageIds={searchMatchingMessageIds}
              searchQuery={searchQuery}
              threadUnreadCounts={threadUnreadCounts}
              unfollowThreadById={unfollowThreadById}
            />
          ))}
        </section>
      ))}
    </div>
  );
});

type TimelineRenderRowViewProps = Omit<
  TimelineMessageListProps,
  "firstUnreadMessageId" | "messages" | "personaLookup"
> & {
  allMessages?: TimelineMessage[];
  row: TimelineRenderRow;
};

const TimelineRenderRowView = React.memo(function TimelineRenderRowView({
  agentPubkeys,
  allMessages,
  channelId,
  channelName,
  channelType,
  currentPubkey,
  followThreadById,
  highlightedMessageId = null,
  isFollowingThreadById,
  isSendingVideoReviewComment = false,
  messageFooters,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  row,
  threadUnreadCounts,
  unfollowThreadById,
}: TimelineRenderRowViewProps) {
  const messageForContext = row.type === "message" ? row.message : null;
  const videoReviewContext = React.useMemo(() => {
    if (!allMessages || !messageForContext) {
      return undefined;
    }

    return buildVideoReviewContextForMessage({
      channelId,
      channelName,
      channelType,
      comments: buildVideoReviewCommentsForRoot(
        allMessages,
        messageForContext.id,
      ),
      isSendingVideoReviewComment,
      message: messageForContext,
      onSendVideoReviewComment,
      onToggleReaction,
      profiles,
    });
  }, [
    allMessages,
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messageForContext,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
  ]);

  if (row.type === "day") {
    return <DayDivider label={row.label} />;
  }

  if (row.type === "unread") {
    return <UnreadDivider />;
  }

  const { message, summary } = row;

  if (message.kind === KIND_SYSTEM_MESSAGE) {
    const footer = messageFooters?.[message.id] ?? null;
    return (
      <div className="flex flex-col gap-1">
        <SystemMessageRow
          message={message}
          currentPubkey={currentPubkey}
          onToggleReaction={onToggleReaction}
          profiles={profiles}
        />
        {footer}
      </div>
    );
  }

  if (summary && onReply) {
    const footer = messageFooters?.[message.id] ?? null;
    const isHighlighted = message.id === highlightedMessageId;
    return (
      <div
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
  const footer = messageFooters?.[message.id] ?? null;

  return (
    <div className="flex flex-col gap-1">
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
        videoReviewContext={videoReviewContext}
      />
      {footer}
    </div>
  );
});
