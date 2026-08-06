import * as React from "react";

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { THREAD_REPLY_ROW_MARGIN_INLINE_REM } from "@/features/messages/lib/threadTreeLayout";
import { timelineMessagesEqual } from "@/features/messages/lib/structureShareTimelineMessages";
import type { buildVideoReviewContextForMessage } from "@/features/messages/lib/videoReviewContext";
import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { MessageRow } from "./MessageRow";
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
  onToggleReaction?: ToggleReaction;
  profiles?: UserProfileLookup;
  searchActiveMessageId?: string | null;
  searchMatchingMessageIds?: Set<string>;
  searchQuery?: string;
  threadUnreadCounts?: ReadonlyMap<string, number>;
  unfollowThreadById?: (rootId: string) => void;
  videoReviewContext: ReturnType<typeof buildVideoReviewContextForMessage>;
};

function threadSummariesEqual(
  a: MainTimelineEntry["summary"],
  b: MainTimelineEntry["summary"],
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (
    a.threadHeadId !== b.threadHeadId ||
    a.replyCount !== b.replyCount ||
    a.lastReplyAt !== b.lastReplyAt ||
    a.participants.length !== b.participants.length
  ) {
    return false;
  }
  for (let i = 0; i < a.participants.length; i += 1) {
    const left = a.participants[i];
    const right = b.participants[i];
    if (
      left.id !== right.id ||
      left.author !== right.author ||
      left.avatarUrl !== right.avatarUrl
    ) {
      return false;
    }
  }
  return true;
}

function MessageRowItemBase({
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
          depth={message.depth}
          message={message}
          onOpenThread={onOpenThread}
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

/**
 * Wrapper memo: live timeline reformats rebuild `MainTimelineEntry` shells and
 * `TimelineMessage` objects even when row content is unchanged. Compare the
 * message by value (and summary by value) so unchanged *visible* rows skip
 * recreating inline follow handlers / MessageRow props — cheaper than
 * structure-sharing every formatted row on the format path.
 */
export const MessageRowItem = React.memo(
  MessageRowItemBase,
  (prev, next) =>
    prev.channelId === next.channelId &&
    prev.currentPubkey === next.currentPubkey &&
    timelineMessagesEqual(prev.entry.message, next.entry.message) &&
    threadSummariesEqual(prev.entry.summary, next.entry.summary) &&
    prev.followThreadById === next.followThreadById &&
    prev.footer === next.footer &&
    prev.highlightedMessageId === next.highlightedMessageId &&
    prev.huddleMemberPubkeys === next.huddleMemberPubkeys &&
    prev.huddleMemberPubkeysPending === next.huddleMemberPubkeysPending &&
    prev.hideAgentAccessBadges === next.hideAgentAccessBadges &&
    prev.isContinuation === next.isContinuation &&
    prev.isFollowedByContinuation === next.isFollowedByContinuation &&
    prev.isFollowingThreadById === next.isFollowingThreadById &&
    prev.isUnread === next.isUnread &&
    prev.playEntrance === next.playEntrance &&
    prev.onEntranceComplete === next.onEntranceComplete &&
    prev.onDelete === next.onDelete &&
    prev.onEdit === next.onEdit &&
    prev.onMarkUnread === next.onMarkUnread &&
    prev.onMarkRead === next.onMarkRead &&
    prev.onReply === next.onReply &&
    prev.onOpenThread === next.onOpenThread &&
    prev.onToggleReaction === next.onToggleReaction &&
    prev.profiles === next.profiles &&
    prev.searchActiveMessageId === next.searchActiveMessageId &&
    prev.searchMatchingMessageIds === next.searchMatchingMessageIds &&
    prev.searchQuery === next.searchQuery &&
    prev.threadUnreadCounts === next.threadUnreadCounts &&
    prev.unfollowThreadById === next.unfollowThreadById &&
    prev.videoReviewContext === next.videoReviewContext,
);
