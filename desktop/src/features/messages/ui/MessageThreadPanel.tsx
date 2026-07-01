import * as React from "react";
import { ArrowDown } from "lucide-react";

import {
  buildThreadSummaryFromVisibleEntries,
  hasNestedThreadBranches,
  type MainTimelineEntry,
} from "@/features/messages/lib/threadPanel";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { cn } from "@/shared/lib/cn";
import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { AuxiliaryPanelBody } from "@/shared/layout/AuxiliaryPanel";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";
import { MessageComposer } from "./MessageComposer";
import { MessageRow, type ThreadDepthGuideAction } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { TypingIndicatorRow } from "./TypingIndicatorRow";
import { UnreadDivider } from "./UnreadDivider";
import { useComposerHeightPadding } from "./useComposerHeightPadding";
import { useAnchoredScroll } from "./useAnchoredScroll";
import { selectDeferredListRenderState } from "@/features/messages/lib/timelineSnapshot";

type MessageThreadPanelProps = {
  agentPubkeys?: ReadonlySet<string>;
  channel: Channel | null;
  channelId: string | null;
  channelName: string;
  currentPubkey?: string;
  disabled?: boolean;
  firstUnreadReplyId?: string | null;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  layout?: "standalone" | "split";
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  isSending: boolean;
  isSinglePanelView?: boolean;
  onCancelEdit?: () => void;
  onCancelReply: () => void;
  onClose: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditLastOwnMessage?: () => boolean;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onExpandReplies: (message: TimelineMessage) => void;
  onScrollTargetResolved: () => void;
  onSelectReplyTarget: (message: TimelineMessage) => void;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  replyTargetMessage: TimelineMessage | null;
  scrollTargetId: string | null;
  threadHead: TimelineMessage | null;
  threadReplies: MainTimelineEntry[];
  threadUnreadCount?: number;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  threadTypingPubkeys: string[];
  threadHeadVideoReviewContext?: VideoReviewContext;
  toolbarExtraActions?: React.ReactNode;
  widthPx: number;
  transparentChrome?: boolean;
  isFollowingThread?: boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
};

const EMPTY_THREAD_REPLIES: MainTimelineEntry[] = [];
const THREAD_PANEL_MESSAGE_GUTTER_CLASS = "px-2";
const THREAD_PANEL_COMPOSER_GUTTER_CLASS = "px-5";
const THREAD_PANEL_SUMMARY_INDENT_OFFSET_REM = -0.125;
type MessageThreadPanelSkeletonProps = {
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  widthPx: number;
  transparentChrome?: boolean;
};

function hasLaterVisibleSibling(
  entries: readonly MainTimelineEntry[],
  entryIndex: number,
): boolean {
  const depth = entries[entryIndex]?.message.depth;
  if (depth == null) {
    return false;
  }

  for (let index = entryIndex + 1; index < entries.length; index += 1) {
    const nextDepth = entries[index].message.depth;
    if (nextDepth <= depth) {
      return nextDepth === depth;
    }
  }

  return false;
}

function getActiveContinuationDepths({
  ancestors,
  entries,
  index,
  message,
}: {
  ancestors: readonly { index: number; message: TimelineMessage }[];
  entries: readonly MainTimelineEntry[];
  index: number;
  message: TimelineMessage;
}): number[] {
  const depths: number[] = [];

  for (const ancestor of ancestors) {
    if (ancestor.message.depth === 0) {
      continue;
    }

    const childDepth = ancestor.message.depth + 1;
    const pathChild =
      message.depth === childDepth
        ? { index, message }
        : ancestors.find((candidate) => candidate.message.depth === childDepth);

    if (pathChild && hasLaterVisibleSibling(entries, pathChild.index)) {
      depths.push(ancestor.message.depth);
    }
  }

  return depths;
}

function ThreadMessageSkeleton({ isHead = false }: { isHead?: boolean }) {
  return (
    <article className="relative flex items-start gap-2.5 rounded-2xl px-3 py-2">
      <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
      <div className="-mt-1 min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
          <Skeleton className="h-[15px] w-28" />
          <Skeleton className="h-3 w-16" />
        </div>
        <div className="mt-1 space-y-1.5 pb-2">
          <Skeleton className="h-4 w-full" />
          <Skeleton className={isHead ? "h-4 w-4/5" : "h-4 w-2/3"} />
        </div>
        <div className="flex items-center gap-4">
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
        </div>
      </div>
    </article>
  );
}

function ThreadComposerSkeleton() {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
      <div className="pointer-events-auto">
        <div
          className={cn(
            "relative z-10 shrink-0 bg-transparent pb-2 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        >
          <div className="relative isolate rounded-2xl border border-border/50 bg-background/80 px-3 pb-2 pt-3 shadow-none backdrop-blur-md sm:px-4">
            <Skeleton className="h-5 w-48 max-w-full" />
            <div className="mt-4 flex items-center gap-2">
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="ml-auto h-8 w-20 rounded-full" />
            </div>
          </div>
        </div>
        <div
          className={cn(
            "h-7 bg-background pb-1 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        />
      </div>
    </div>
  );
}

export function MessageThreadPanelSkeleton({
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  widthPx,
  transparentChrome = false,
}: MessageThreadPanelSkeletonProps) {
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const threadHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup
        backButtonAriaLabel="Back to conversation"
        onBack={isSinglePanelView ? onClose : undefined}
      >
        <AuxiliaryPanelTitle>Thread</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
    </>
  );

  const threadBody = (
    <AuxiliaryPanelBody
      className="overflow-y-auto overflow-x-hidden overscroll-contain pb-24"
      data-testid="message-thread-loading"
    >
      <div
        className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-1 pt-0")}
        data-testid="message-thread-head-loading"
      >
        <ThreadMessageSkeleton isHead />
      </div>
      <div
        className={cn(
          "space-y-2.5 pb-3 pt-1",
          THREAD_PANEL_MESSAGE_GUTTER_CLASS,
        )}
      >
        <ThreadMessageSkeleton />
        <ThreadMessageSkeleton />
        <div className="ml-[58px] flex items-center gap-1.5 pt-0.5">
          <Skeleton className="h-7 w-7 rounded-full" />
          <Skeleton className="h-7 w-7 rounded-full" />
          <Skeleton className="h-4 w-28 rounded-full" />
        </div>
      </div>
    </AuxiliaryPanelBody>
  );

  return (
    <AuxiliaryPanel
      className="relative"
      footer={<ThreadComposerSkeleton />}
      header={
        <AuxiliaryPanelHeader>{threadHeaderContent}</AuxiliaryPanelHeader>
      }
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="message-thread-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
    >
      {threadBody}
    </AuxiliaryPanel>
  );
}

export function MessageThreadPanel({
  agentPubkeys,
  channel,
  channelId,
  channelName,
  currentPubkey,
  disabled = false,
  firstUnreadReplyId,
  huddleMemberPubkeys,
  huddleMemberPubkeysPending = false,
  layout = "standalone",
  editTarget,
  isSending,
  isSinglePanelView = false,
  isFollowingThread,
  isMessageUnreadById,
  onCancelEdit,
  onCancelReply,
  onClose,
  onDelete,
  onEdit,
  onEditLastOwnMessage,
  onEditSave,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onExpandReplies,
  onScrollTargetResolved,
  onSelectReplyTarget,
  onSend,
  onToggleReaction,
  onUnfollowThread,
  profiles,
  replyTargetMessage,
  scrollTargetId,
  threadHead,
  threadHeadVideoReviewContext,
  threadReplies,
  threadUnreadCount,
  threadReplyUnreadCounts,
  threadTypingPubkeys,
  toolbarExtraActions,
  widthPx,
  transparentChrome = false,
}: MessageThreadPanelProps) {
  const threadBodyRef = React.useRef<HTMLDivElement>(null);
  const threadContentRef = React.useRef<HTMLDivElement>(null);
  const threadComposerWrapperRef = React.useRef<HTMLDivElement>(null);
  const [hoveredCollapseBranchId, setHoveredCollapseBranchId] = React.useState<
    string | null
  >(null);
  const [collapsedThreadHeadId, setCollapsedThreadHeadId] = React.useState<
    string | null
  >(null);
  const isOverlay = useIsThreadPanelOverlay();
  const threadHeadId = threadHead?.id ?? null;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);
  useComposerHeightPadding(
    threadBodyRef,
    threadComposerWrapperRef,
    isSinglePanelView,
  );

  const collapseThreadHeadReplies = React.useCallback(() => {
    if (!threadHeadId) {
      return;
    }

    setHoveredCollapseBranchId(null);
    setCollapsedThreadHeadId(threadHeadId);
  }, [threadHeadId]);
  const expandThreadHeadReplies = React.useCallback(() => {
    setHoveredCollapseBranchId(null);
    setCollapsedThreadHeadId(null);
  }, []);
  const handleCollapseBranchHoverChange = React.useCallback(
    (message: TimelineMessage, hovered: boolean) => {
      setHoveredCollapseBranchId((current) => {
        if (hovered) {
          return message.id;
        }

        return current === message.id ? null : current;
      });
    },
    [],
  );
  const handleCollapseDepthGuide = React.useCallback(
    (message: TimelineMessage) => {
      if (message.id === threadHeadId) {
        collapseThreadHeadReplies();
        return;
      }

      onExpandReplies(message);
    },
    [collapseThreadHeadReplies, onExpandReplies, threadHeadId],
  );

  const composerReplyTarget =
    replyTargetMessage && threadHead && replyTargetMessage.id !== threadHead.id
      ? {
          author: replyTargetMessage.author,
          body: replyTargetMessage.body,
          id: replyTargetMessage.id,
        }
      : null;

  const deferredThreadReplies = React.useDeferredValue(
    threadReplies,
    EMPTY_THREAD_REPLIES,
  );
  const isRepliesPending = deferredThreadReplies !== threadReplies;
  const scrollTargetIsVisibleReply = React.useMemo(
    () =>
      scrollTargetId !== null &&
      scrollTargetId !== threadHeadId &&
      deferredThreadReplies.some(
        (entry) => entry.message.id === scrollTargetId,
      ),
    [deferredThreadReplies, scrollTargetId, threadHeadId],
  );
  const isThreadHeadRepliesCollapsed =
    collapsedThreadHeadId === threadHeadId && !scrollTargetIsVisibleReply;

  React.useLayoutEffect(() => {
    if (scrollTargetIsVisibleReply && collapsedThreadHeadId === threadHeadId) {
      setCollapsedThreadHeadId(null);
    }
  }, [collapsedThreadHeadId, scrollTargetIsVisibleReply, threadHeadId]);

  // Which of the three states the reply region paints this frame. Delegated to
  // a pure helper so the "don't flash empty over an incoming list" rule is
  // covered in the lib test suite (see selectDeferredListRenderState).
  const repliesRenderState = selectDeferredListRenderState(
    deferredThreadReplies.length,
    threadReplies.length,
  );
  const threadHeadSummary = React.useMemo(() => {
    if (!threadHeadId) {
      return null;
    }

    return buildThreadSummaryFromVisibleEntries(
      threadHeadId,
      deferredThreadReplies,
    );
  }, [deferredThreadReplies, threadHeadId]);
  const visibleThreadHeadSummary = isThreadHeadRepliesCollapsed
    ? threadHeadSummary
    : null;

  const threadMessages = React.useMemo(
    () => deferredThreadReplies.map((entry) => entry.message),
    [deferredThreadReplies],
  );
  const shouldShowThreadBranchGuides = React.useMemo(
    () => hasNestedThreadBranches(deferredThreadReplies),
    [deferredThreadReplies],
  );
  const highlightedBranch = React.useMemo(() => {
    if (!hoveredCollapseBranchId) {
      return null;
    }

    if (hoveredCollapseBranchId === threadHeadId) {
      return {
        depth: 0,
        endIndex: deferredThreadReplies.length - 1,
        id: hoveredCollapseBranchId,
        startIndex: -1,
      };
    }

    const startIndex = deferredThreadReplies.findIndex(
      (entry) => entry.message.id === hoveredCollapseBranchId,
    );
    if (startIndex < 0) {
      return null;
    }

    const depth = deferredThreadReplies[startIndex].message.depth;
    let endIndex = startIndex;
    while (
      endIndex + 1 < deferredThreadReplies.length &&
      deferredThreadReplies[endIndex + 1].message.depth > depth
    ) {
      endIndex += 1;
    }

    return {
      depth,
      endIndex,
      id: hoveredCollapseBranchId,
      startIndex,
    };
  }, [deferredThreadReplies, hoveredCollapseBranchId, threadHeadId]);
  const threadReplyRenderItems = React.useMemo(() => {
    if (!threadHead) {
      return [];
    }

    const ancestorStack: { index: number; message: TimelineMessage }[] = [
      { index: -1, message: threadHead },
    ];

    return deferredThreadReplies.map((entry, index) => {
      while (
        ancestorStack.length > 0 &&
        ancestorStack[ancestorStack.length - 1].message.depth >=
          entry.message.depth
      ) {
        ancestorStack.pop();
      }

      const ancestors = [...ancestorStack];
      const continuationDepths = getActiveContinuationDepths({
        ancestors,
        entries: deferredThreadReplies,
        index,
        message: entry.message,
      });
      const collapseDepthGuideAncestors = ancestors.filter((ancestor) =>
        continuationDepths.includes(ancestor.message.depth),
      );
      const collapseDepthGuideActions: ThreadDepthGuideAction[] | undefined =
        collapseDepthGuideAncestors.length > 0
          ? collapseDepthGuideAncestors.map((ancestor) => ({
              active:
                hoveredCollapseBranchId === ancestor.message.id &&
                entry.message.depth === ancestor.message.depth + 1,
              depth: ancestor.message.depth,
              label:
                ancestor.message.id === threadHead.id
                  ? "Collapse thread"
                  : "Collapse replies",
              message: ancestor.message,
            }))
          : undefined;
      const nextEntry = deferredThreadReplies[index + 1];
      const connectsToVisibleChild =
        nextEntry != null && nextEntry.message.depth > entry.message.depth;

      if (connectsToVisibleChild && !entry.summary) {
        ancestorStack.push({ index, message: entry.message });
      }

      return {
        collapseDepthGuideActions,
        connectsToVisibleChild,
        continuationDepths,
        entry,
        index,
      };
    });
  }, [deferredThreadReplies, hoveredCollapseBranchId, threadHead]);

  const { isAtBottom, newMessageCount, onScroll, scrollToBottom } =
    useAnchoredScroll({
      channelId: threadHeadId,
      contentRef: threadContentRef,
      isLoading: repliesRenderState === "pending",
      messages: threadMessages,
      onTargetReached: onScrollTargetResolved,
      scrollContainerRef: threadBodyRef,
      targetMessageId: scrollTargetId,
    });

  if (!threadHead) {
    return null;
  }

  const threadScrollRegion = (
    <AuxiliaryPanelBody
      className="overflow-y-auto overflow-x-hidden overscroll-contain pb-24"
      data-buzz-conversation-scroll
      data-testid="message-thread-body"
      onScroll={onScroll}
      ref={threadBodyRef}
    >
      <div ref={threadContentRef}>
        <div
          className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-1 pt-0")}
          data-testid="message-thread-head"
        >
          <div className="rounded-2xl">
            <MessageRow
              actionBarPlacement="inside"
              agentPubkeys={agentPubkeys}
              channelId={channelId}
              huddleMemberPubkeys={huddleMemberPubkeys}
              huddleMemberPubkeysPending={huddleMemberPubkeysPending}
              isFollowingThread={isFollowingThread}
              isUnread={isMessageUnreadById?.(threadHead.id)}
              layoutVariant="thread-reply"
              message={threadHead}
              onDelete={
                onDelete &&
                canManageMessageForCurrentUser(
                  threadHead,
                  currentPubkey,
                  profiles,
                )
                  ? onDelete
                  : undefined
              }
              onEdit={
                onEdit &&
                canManageMessageForCurrentUser(
                  threadHead,
                  currentPubkey,
                  profiles,
                )
                  ? onEdit
                  : undefined
              }
              onFollowThread={
                onFollowThread ? (_msg) => onFollowThread() : undefined
              }
              onMarkUnread={onMarkUnread}
              onMarkRead={onMarkRead}
              onToggleReaction={onToggleReaction}
              onUnfollowThread={
                onUnfollowThread ? (_msg) => onUnfollowThread() : undefined
              }
              profiles={profiles}
              showDepthGuides={shouldShowThreadBranchGuides}
              videoReviewContext={threadHeadVideoReviewContext}
            />
          </div>
        </div>

        <div
          className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-3 pt-0")}
          data-testid="message-thread-replies"
        >
          {repliesRenderState === "list" ? (
            visibleThreadHeadSummary ? (
              <div
                className="space-y-0"
                data-render-pending={isRepliesPending ? "true" : undefined}
              >
                <MessageThreadSummaryRow
                  depth={threadHead.depth}
                  message={threadHead}
                  onOpenThread={expandThreadHeadReplies}
                  summary={visibleThreadHeadSummary}
                  summaryIndentOffsetRem={
                    THREAD_PANEL_SUMMARY_INDENT_OFFSET_REM
                  }
                  unreadCount={threadUnreadCount}
                />
              </div>
            ) : (
              <div
                className="space-y-0"
                data-render-pending={isRepliesPending ? "true" : undefined}
              >
                {threadReplyRenderItems.map((item) => {
                  const {
                    collapseDepthGuideActions,
                    connectsToVisibleChild,
                    continuationDepths,
                    entry,
                    index,
                  } = item;
                  const showUnreadDivider =
                    index > 0 && entry.message.id === firstUnreadReplyId;
                  const isHighlightedBranchOwner =
                    highlightedBranch?.id === entry.message.id;
                  const isInsideHighlightedBranch =
                    highlightedBranch != null &&
                    index > highlightedBranch.startIndex &&
                    index <= highlightedBranch.endIndex;
                  const isDirectChildOfHighlightedBranch =
                    isInsideHighlightedBranch &&
                    highlightedBranch != null &&
                    index > highlightedBranch.startIndex &&
                    index <= highlightedBranch.endIndex &&
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
                      key={entry.message.renderKey ?? entry.message.id}
                    >
                      {showUnreadDivider ? <UnreadDivider /> : null}
                      <MessageRow
                        agentPubkeys={agentPubkeys}
                        channelId={channelId}
                        collapseDepthGuideActions={collapseDepthGuideActions}
                        collapseDescendantsLabel="Collapse replies"
                        connectDescendants={
                          shouldShowThreadBranchGuides && connectsToVisibleChild
                        }
                        depthGuideDepths={
                          shouldShowThreadBranchGuides
                            ? continuationDepths
                            : undefined
                        }
                        highlightDescendantRail={
                          shouldShowThreadBranchGuides &&
                          isHighlightedBranchOwner &&
                          connectsToVisibleChild
                        }
                        highlightReplyConnector={
                          shouldShowThreadBranchGuides &&
                          isDirectChildOfHighlightedBranch
                        }
                        highlightThreadLineDepths={highlightedLineDepths}
                        hoverBackground={!entry.summary}
                        huddleMemberPubkeys={huddleMemberPubkeys}
                        huddleMemberPubkeysPending={huddleMemberPubkeysPending}
                        isUnread={isMessageUnreadById?.(entry.message.id)}
                        layoutVariant="thread-reply"
                        message={entry.message}
                        onCollapseDepthGuide={handleCollapseDepthGuide}
                        onCollapseDepthGuideHoverChange={
                          handleCollapseBranchHoverChange
                        }
                        onCollapseDescendants={
                          shouldShowThreadBranchGuides &&
                          connectsToVisibleChild &&
                          !entry.summary
                            ? onExpandReplies
                            : undefined
                        }
                        onCollapseDescendantsHoverChange={
                          handleCollapseBranchHoverChange
                        }
                        onDelete={
                          onDelete &&
                          canManageMessageForCurrentUser(
                            entry.message,
                            currentPubkey,
                            profiles,
                          )
                            ? onDelete
                            : undefined
                        }
                        onEdit={
                          onEdit &&
                          canManageMessageForCurrentUser(
                            entry.message,
                            currentPubkey,
                            profiles,
                          )
                            ? onEdit
                            : undefined
                        }
                        onMarkUnread={onMarkUnread}
                        onMarkRead={onMarkRead}
                        onReply={onSelectReplyTarget}
                        onToggleReaction={onToggleReaction}
                        profiles={profiles}
                        showDepthGuides={shouldShowThreadBranchGuides}
                      />
                      {entry.summary ? (
                        <MessageThreadSummaryRow
                          collapseDepthGuideActions={collapseDepthGuideActions}
                          depth={entry.message.depth}
                          depthGuideDepths={
                            shouldShowThreadBranchGuides
                              ? continuationDepths
                              : undefined
                          }
                          highlightThreadLineDepths={highlightedLineDepths}
                          message={entry.message}
                          onCollapseDepthGuide={handleCollapseDepthGuide}
                          onCollapseDepthGuideHoverChange={
                            handleCollapseBranchHoverChange
                          }
                          onOpenThread={onExpandReplies}
                          summary={entry.summary}
                          summaryIndentOffsetRem={
                            THREAD_PANEL_SUMMARY_INDENT_OFFSET_REM
                          }
                          showDepthGuides={shouldShowThreadBranchGuides}
                          unreadCount={threadReplyUnreadCounts?.get(
                            entry.message.id,
                          )}
                        />
                      ) : null}
                    </div>
                  );
                })}
              </div>
            )
          ) : repliesRenderState === "empty" ? (
            // Only show the empty state when the thread is GENUINELY empty.
            // Keying off `deferredThreadReplies` would flash "No replies" for a
            // frame while a non-empty list streams in on the deferred commit.
            <div className="rounded-2xl border border-dashed border-border/70 bg-card/40 px-4 py-6 text-center">
              <p className="text-sm font-medium text-foreground/80">
                No replies in this branch yet
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                Reply in the thread to continue this branch.
              </p>
            </div>
          ) : // "pending": deferred list is empty but the live list has content —
          // rows are streaming in on the deferred commit. Paint nothing rather
          // than flashing the empty state.
          null}
        </div>
      </div>
    </AuxiliaryPanelBody>
  );

  const threadFooter = (
    <>
      {!isAtBottom ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-36 z-50 flex justify-center px-4">
          <Button
            className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-2xs font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
            data-testid="thread-scroll-to-latest"
            onClick={() => scrollToBottom("smooth")}
            size="sm"
            type="button"
            variant="outline"
          >
            <ArrowDown aria-hidden />
            {newMessageCount > 0
              ? `${newMessageCount} new message${newMessageCount === 1 ? "" : "s"}`
              : "Jump to latest"}
          </Button>
        </div>
      ) : null}

      <div
        className="pointer-events-none absolute inset-x-0 bottom-0 z-40"
        data-testid="thread-composer-overlay"
        ref={threadComposerWrapperRef}
      >
        <div className="pointer-events-auto">
          <MessageComposer
            channelId={channelId}
            channelName={channelName}
            channelType={channel?.channelType ?? null}
            containerClassName={THREAD_PANEL_COMPOSER_GUTTER_CLASS}
            disabled={disabled || isSending || !channelId}
            draftKey={`thread:${threadHead.id}`}
            editTarget={editTarget}
            isSending={isSending}
            onCancelEdit={onCancelEdit}
            onCancelReply={composerReplyTarget ? onCancelReply : undefined}
            onEditLastOwnMessage={onEditLastOwnMessage}
            onEditSave={onEditSave}
            onSend={onSend}
            placeholder={`Reply in thread to ${threadHead.author}`}
            profiles={profiles}
            replyTarget={composerReplyTarget}
            typingParentEventId={threadHead.id}
            typingRootEventId={threadHead.rootId}
          />
          <div
            className={cn(
              "min-h-8 bg-background pb-1.5 pt-0",
              THREAD_PANEL_COMPOSER_GUTTER_CLASS,
            )}
          >
            <div className="mx-auto flex h-full w-full max-w-4xl items-center gap-2 overflow-visible">
              {toolbarExtraActions ? (
                <div className="flex min-w-0 flex-1 overflow-visible">
                  {toolbarExtraActions}
                </div>
              ) : null}
              {threadTypingPubkeys.length > 0 ? (
                <TypingIndicatorRow
                  channel={channel}
                  className="min-w-0 flex-1 py-0 pl-[calc(0.75rem+1px)] pr-0 sm:pl-[calc(1rem+1px)]"
                  currentPubkey={currentPubkey}
                  profiles={profiles}
                  typingPubkeys={threadTypingPubkeys}
                  variant="activity"
                />
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </>
  );

  const threadHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup
        backButtonAriaLabel="Back to conversation"
        backButtonTestId="message-thread-back"
        onBack={isSinglePanelView ? onClose : undefined}
      >
        <AuxiliaryPanelTitle>Thread</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
    </>
  );

  return (
    <AuxiliaryPanel
      className="relative"
      footer={threadFooter}
      header={
        <AuxiliaryPanelHeader>{threadHeaderContent}</AuxiliaryPanelHeader>
      }
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="message-thread-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
    >
      {threadScrollRegion}
    </AuxiliaryPanel>
  );
}
