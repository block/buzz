import * as React from "react";

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { THREAD_REPLY_ROW_MARGIN_INLINE_REM } from "@/features/messages/lib/threadTreeLayout";
import type { buildVideoReviewContextForMessage } from "@/features/messages/lib/videoReviewContext";
import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { MessageRow } from "./MessageRow";
import { MessageThreadRow } from "./MessageThreadRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";

type ToggleReaction = (
  message: TimelineMessage,
  emoji: string,
  remove: boolean,
) => Promise<void>;

type SystemRowProps = {
  currentPubkey?: string;
  entries?: MainTimelineEntry[];
  entry?: MainTimelineEntry;
  footer: React.ReactNode;
  onToggleReaction?: ToggleReaction;
  profiles?: UserProfileLookup;
  ownerProfiles?: UserProfileLookup;
};

export function SystemRow({
  currentPubkey,
  entries,
  entry,
  footer,
  onToggleReaction,
  profiles,
  ownerProfiles,
}: SystemRowProps) {
  const systemEntries = entries ?? (entry ? [entry] : []);
  const firstEntry = systemEntries[0];
  const groupedMessages = React.useMemo(
    () => entries?.map((systemEntry) => systemEntry.message),
    [entries],
  );
  if (!firstEntry) return null;

  return (
    <div className="flex flex-col gap-1 pb-2.5">
      <SystemMessageRow
        groupedMessages={groupedMessages}
        message={firstEntry.message}
        currentPubkey={currentPubkey}
        onToggleReaction={onToggleReaction}
        profiles={profiles}
        ownerProfiles={ownerProfiles}
      />
      {footer}
    </div>
  );
}

type MessageRowItemProps = {
  channelId?: string | null;
  currentPubkey?: string;
  entry: MainTimelineEntry;
  followThreadById?: (rootId: string) => void;
  footer: React.ReactNode;
  highlightedMessageId?: string | null;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  hideAgentAccessBadges?: boolean;
  isContinuation?: boolean;
  isFollowedByContinuation?: boolean;
  isFollowingThreadById?: (rootId: string) => boolean;
  isUnread?: boolean;
  playEntrance?: boolean;
  onEntranceComplete?: (messageId: string) => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  onOpenThread?: (message: TimelineMessage) => void;
  onOpenInlineThreadPanel?: (message: TimelineMessage) => void;
  onToggleReaction?: ToggleReaction;
  profiles?: UserProfileLookup;
  searchActiveMessageId?: string | null;
  searchMatchingMessageIds?: Set<string>;
  searchQuery?: string;
  threadUnreadCounts?: ReadonlyMap<string, number>;
  unfollowThreadById?: (rootId: string) => void;
  videoReviewContext: ReturnType<typeof buildVideoReviewContextForMessage>;
};

export function MessageRowItem({
  channelId,
  currentPubkey,
  entry,
  followThreadById,
  footer,
  highlightedMessageId,
  huddleMemberPubkeys,
  huddleMemberPubkeysPending,
  hideAgentAccessBadges,
  isContinuation = false,
  isFollowedByContinuation = false,
  isFollowingThreadById,
  isUnread,
  playEntrance = false,
  onEntranceComplete,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
  onReply,
  onOpenThread,
  onOpenInlineThreadPanel,
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
  const canManage = canManageMessageForCurrentUser(
    message,
    currentPubkey,
    profiles,
  );
  const canDelete = canManage && onDelete ? onDelete : undefined;
  const canEdit = canManage && onEdit ? onEdit : undefined;

  if (summary && onOpenThread) {
    const isHighlighted = message.id === highlightedMessageId;
    const inlineThreadOpen = Boolean(entry.inlineThread);
    return (
      <div
        className={cn(
          "group/message relative mx-1 mb-1 flex flex-col gap-0 rounded-2xl px-0 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
          isHighlighted &&
            "-mx-4 px-4 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-6",
        )}
      >
        <MessageRow
          channelId={channelId}
          highlighted={false}
          hoverBackground={false}
          huddleMemberPubkeys={huddleMemberPubkeys}
          huddleMemberPubkeysPending={huddleMemberPubkeysPending}
          hideAgentAccessBadge={hideAgentAccessBadges}
          isFollowingThread={
            isFollowingThreadById
              ? isFollowingThreadById(message.id)
              : undefined
          }
          isUnread={isUnread}
          isContinuation={isContinuation}
          playEntrance={playEntrance}
          onEntranceComplete={onEntranceComplete}
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
          actionLabel={inlineThreadOpen ? "Collapse inline replies" : undefined}
          depth={message.depth}
          message={message}
          onOpenThread={onOpenThread}
          onSecondaryAction={
            inlineThreadOpen && onOpenInlineThreadPanel
              ? onOpenInlineThreadPanel
              : undefined
          }
          secondaryActionLabel="Open in thread"
          showDepthGuides={false}
          summary={summary}
          summaryIndentOffsetRem={-THREAD_REPLY_ROW_MARGIN_INLINE_REM}
          unreadCount={threadUnreadCounts?.get(message.id)}
        />
        {footer}
      </div>
    );
  }

  const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
  const isSearchActive = message.id === searchActiveMessageId;

  return (
    <div
      className={cn(
        "flex flex-col gap-1",
        isFollowedByContinuation ? "pb-0" : "pb-2.5",
      )}
    >
      <MessageRow
        channelId={channelId}
        highlighted={message.id === highlightedMessageId || isSearchActive}
        huddleMemberPubkeys={huddleMemberPubkeys}
        huddleMemberPubkeysPending={huddleMemberPubkeysPending}
        hideAgentAccessBadge={hideAgentAccessBadges}
        isContinuation={isContinuation}
        isUnread={isUnread}
        playEntrance={playEntrance}
        onEntranceComplete={onEntranceComplete}
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

type InlineThreadReplyRowItemProps = {
  channelId?: string | null;
  currentPubkey?: string;
  entry: MainTimelineEntry;
  footer: React.ReactNode;
  hideAgentAccessBadges?: boolean;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  isContinuation?: boolean;
  isFollowedByContinuation?: boolean;
  isUnread?: boolean;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onExpandInlineThreadReplies?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onOpenInlineThreadPanel?: (message: TimelineMessage) => void;
  onToggleReaction?: ToggleReaction;
  profiles?: UserProfileLookup;
  rootEntry: MainTimelineEntry;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  videoReviewContext: ReturnType<typeof buildVideoReviewContextForMessage>;
};

export function InlineThreadReplyRowItem({
  channelId,
  currentPubkey,
  entry,
  footer,
  hideAgentAccessBadges,
  huddleMemberPubkeys,
  huddleMemberPubkeysPending,
  isContinuation = false,
  isFollowedByContinuation = false,
  isUnread,
  onDelete,
  onEdit,
  onExpandInlineThreadReplies,
  onMarkRead,
  onMarkUnread,
  onOpenInlineThreadPanel,
  onToggleReaction,
  profiles,
  rootEntry,
  threadReplyUnreadCounts,
  videoReviewContext,
}: InlineThreadReplyRowItemProps) {
  const { message, summary } = entry;
  const canManage = canManageMessageForCurrentUser(
    message,
    currentPubkey,
    profiles,
  );
  const canDelete = canManage && onDelete ? onDelete : undefined;
  const canEdit = canManage && onEdit ? onEdit : undefined;
  return (
    <div
      className={cn(
        "flex flex-col gap-0",
        isFollowedByContinuation ? "pb-0" : "pb-2.5",
        summary &&
          "group/message rounded-2xl px-0 py-0.5 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
      )}
    >
      <MessageThreadRow
        channelId={channelId}
        currentPubkey={currentPubkey}
        hideAgentAccessBadge={hideAgentAccessBadges}
        hoverBackground={!summary}
        huddleMemberPubkeys={huddleMemberPubkeys}
        huddleMemberPubkeysPending={huddleMemberPubkeysPending}
        isContinuation={isContinuation}
        isUnread={isUnread}
        message={message}
        onDelete={canDelete}
        onEdit={canEdit}
        onMarkRead={onMarkRead}
        onMarkUnread={onMarkUnread}
        onToggleReaction={onToggleReaction}
        profiles={profiles}
        showDepthGuides={false}
        videoReviewContext={videoReviewContext}
      />
      {summary ? (
        <MessageThreadSummaryRow
          actionLabel="Show replies"
          depth={message.depth}
          message={message}
          onOpenThread={
            onExpandInlineThreadReplies ??
            (() => onOpenInlineThreadPanel?.(rootEntry.message))
          }
          onSecondaryAction={
            onOpenInlineThreadPanel
              ? () => onOpenInlineThreadPanel(rootEntry.message)
              : undefined
          }
          secondaryActionLabel="Open in thread"
          showDepthGuides={false}
          summary={summary}
          summaryIndentOffsetRem={-THREAD_REPLY_ROW_MARGIN_INLINE_REM}
          unreadCount={threadReplyUnreadCounts?.get(message.id)}
        />
      ) : null}
      {footer}
    </div>
  );
}

export function InlineThreadLoadingRow() {
  return (
    <div
      aria-hidden
      className="pb-2.5 pl-[calc(2.75rem+0.875rem)] pr-3"
      data-testid="inline-thread-replies-loading"
    >
      <div className="h-11 animate-pulse rounded-2xl bg-muted/45" />
    </div>
  );
}
