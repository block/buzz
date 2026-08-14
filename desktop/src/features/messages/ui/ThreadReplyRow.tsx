import * as React from "react";

import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { VideoReviewPresentation } from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { MessageRow, type ThreadDepthGuideAction } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { UnreadDivider } from "./UnreadDivider";

const SUMMARY_INDENT_OFFSET_REM = 0;

export type ThreadReplyRenderItem = {
  collapseDepthGuideActions?: ThreadDepthGuideAction[];
  connectsToVisibleChild: boolean;
  continuationDepths: number[];
  entry: MainTimelineEntry;
  index: number;
  isContinuation: boolean;
};

export type ThreadReplyRowContext = {
  channelId: string | null;
  currentPubkey?: string;
  firstUnreadReplyId?: string | null;
  highlightedBranch: {
    depth: number;
    endIndex: number;
    id: string;
    startIndex: number;
  } | null;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending: boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
  onCollapseDepthGuide: (message: TimelineMessage) => void;
  onCollapseDepthGuideHoverChange: (
    message: TimelineMessage,
    hovered: boolean,
  ) => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onExpandReplies: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onSelectReplyTarget: (message: TimelineMessage) => void;
  onSendToChannel?: (message: TimelineMessage) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  shouldShowThreadBranchGuides: boolean;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  videoReviewPresentation?: VideoReviewPresentation;
};

export const ThreadReplyRow = React.memo(function ThreadReplyRow({
  context,
  item,
}: {
  context: ThreadReplyRowContext;
  item: ThreadReplyRenderItem;
}) {
  const {
    collapseDepthGuideActions,
    connectsToVisibleChild,
    continuationDepths,
    entry,
    index,
    isContinuation,
  } = item;
  const {
    channelId,
    currentPubkey,
    firstUnreadReplyId,
    highlightedBranch,
    huddleMemberPubkeys,
    huddleMemberPubkeysPending,
    isMessageUnreadById,
    onCollapseDepthGuide,
    onCollapseDepthGuideHoverChange,
    onDelete,
    onEdit,
    onExpandReplies,
    onMarkRead,
    onMarkUnread,
    onSelectReplyTarget,
    onSendToChannel,
    onToggleReaction,
    profiles,
    shouldShowThreadBranchGuides,
    threadReplyUnreadCounts,
    videoReviewPresentation,
  } = context;
  const showUnreadDivider =
    index > 0 && entry.message.id === firstUnreadReplyId;
  const isHighlightedBranchOwner = highlightedBranch?.id === entry.message.id;
  const isInsideHighlightedBranch =
    highlightedBranch != null &&
    index > highlightedBranch.startIndex &&
    index <= highlightedBranch.endIndex;
  const isDirectChildOfHighlightedBranch =
    isInsideHighlightedBranch &&
    highlightedBranch != null &&
    entry.message.depth === highlightedBranch.depth + 1;
  const highlightedLineDepths =
    shouldShowThreadBranchGuides &&
    isInsideHighlightedBranch &&
    highlightedBranch
      ? [highlightedBranch.depth]
      : undefined;

  return (
    <div
      className={cn(
        "flex flex-col gap-0",
        entry.summary &&
          "group/message rounded-2xl px-0 py-0.5 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
      )}
    >
      {showUnreadDivider ? <UnreadDivider /> : null}
      <MessageRow
        channelId={channelId}
        currentPubkey={currentPubkey}
        collapseDepthGuideActions={collapseDepthGuideActions}
        collapseDescendantsLabel="Collapse replies"
        connectDescendants={
          shouldShowThreadBranchGuides && connectsToVisibleChild
        }
        depthGuideDepths={
          shouldShowThreadBranchGuides ? continuationDepths : undefined
        }
        highlightDescendantRail={
          shouldShowThreadBranchGuides &&
          isHighlightedBranchOwner &&
          connectsToVisibleChild
        }
        highlightReplyConnector={
          shouldShowThreadBranchGuides && isDirectChildOfHighlightedBranch
        }
        highlightThreadLineDepths={highlightedLineDepths}
        hoverBackground={!entry.summary}
        huddleMemberPubkeys={huddleMemberPubkeys}
        huddleMemberPubkeysPending={huddleMemberPubkeysPending}
        isContinuation={isContinuation}
        isUnread={isMessageUnreadById?.(entry.message.id)}
        layoutVariant="thread-reply"
        message={entry.message}
        onCollapseDepthGuide={onCollapseDepthGuide}
        onCollapseDepthGuideHoverChange={onCollapseDepthGuideHoverChange}
        onCollapseDescendants={
          shouldShowThreadBranchGuides &&
          connectsToVisibleChild &&
          !entry.summary
            ? onExpandReplies
            : undefined
        }
        onCollapseDescendantsHoverChange={onCollapseDepthGuideHoverChange}
        onDelete={
          onDelete &&
          canManageMessageForCurrentUser(entry.message, currentPubkey, profiles)
            ? onDelete
            : undefined
        }
        onEdit={
          onEdit &&
          canManageMessageForCurrentUser(entry.message, currentPubkey, profiles)
            ? onEdit
            : undefined
        }
        onMarkUnread={onMarkUnread}
        onMarkRead={onMarkRead}
        onReply={onSelectReplyTarget}
        onSendToChannel={onSendToChannel}
        onToggleReaction={onToggleReaction}
        profiles={profiles}
        showDepthGuides={shouldShowThreadBranchGuides}
        videoReviewCommentRootId={videoReviewPresentation?.commentRootIdsByMessageId.get(
          entry.message.id,
        )}
        videoReviewContext={videoReviewPresentation?.contextsByMessageId.get(
          entry.message.id,
        )}
      />
      {entry.summary ? (
        <MessageThreadSummaryRow
          collapseDepthGuideActions={collapseDepthGuideActions}
          depth={entry.message.depth}
          depthGuideDepths={
            shouldShowThreadBranchGuides ? continuationDepths : undefined
          }
          highlightThreadLineDepths={highlightedLineDepths}
          message={entry.message}
          onCollapseDepthGuide={onCollapseDepthGuide}
          onCollapseDepthGuideHoverChange={onCollapseDepthGuideHoverChange}
          onOpenThread={onExpandReplies}
          summary={entry.summary}
          summaryIndentOffsetRem={SUMMARY_INDENT_OFFSET_REM}
          showDepthGuides={shouldShowThreadBranchGuides}
          unreadCount={threadReplyUnreadCounts?.get(entry.message.id)}
        />
      ) : null}
    </div>
  );
});
