import * as React from "react";
import { GroupedVirtuoso } from "react-virtuoso";
import type {
  Components,
  GroupedVirtuosoHandle,
  ListRange,
  ScrollerProps,
} from "react-virtuoso";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import { timelineRowReserveStyle } from "@/features/messages/lib/rowHeightEstimate";
import {
  buildTimelineDayGroups,
  buildTimelineItems,
  getTimelineItemKey,
  type TimelineDayGroup,
  type TimelineNonDayItem,
} from "@/features/messages/lib/timelineItems";
import { THREAD_REPLY_ROW_MARGIN_INLINE_REM } from "@/features/messages/lib/threadTreeLayout";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
  hasVideoAttachment,
} from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";
import { UnreadDivider } from "./UnreadDivider";

export type TimelineVirtualizerApi = {
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  scrollToMessage: (
    messageId: string,
    options?: { behavior?: ScrollBehavior },
  ) => boolean;
};

type TimelineMessageListProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelName?: string;
  channelType?: ChannelType | null;
  currentPubkey?: string;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
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
  /** Relay thread summaries keyed by thread root id. Keeps badge rows alive on
   *  the deferred-render fallback — replies usually are not local timeline
   *  rows, so without the relay map every summary row unmounts mid-scrollback. */
  threadSummaries?: ReadonlyMap<string, ChannelWindowThreadSummary>;
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
  /** Existing scroll container, reused by the virtualizer spike so MessageTimeline owns the scroll node. */
  useVirtualizer?: boolean;
  onStartReached?: () => void;
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
  onVirtualizerScroll?: () => void;
  onVirtualizerScrollerChange?: (element: HTMLDivElement | null) => void;
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
  huddleMemberPubkeys,
  huddleMemberPubkeysPending = false,
  isFollowingThreadById,
  isMessageUnreadById,
  messageFooters,
  mainEntries,
  threadSummaries,
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
  useVirtualizer = false,
  onStartReached,
  onAtBottomStateChange,
  onVirtualizerApiChange,
  onVirtualizerRangeChanged,
  onVirtualizerScroll,
  onVirtualizerScrollerChange,
}: TimelineMessageListProps) {
  const entries = React.useMemo(
    () =>
      mainEntries ??
      buildMainTimelineEntries(messages, undefined, threadSummaries, profiles),
    [mainEntries, messages, profiles, threadSummaries],
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

  // The flattened item stream, memoized on the entries and the unread boundary
  // (the unread divider is its own item, so it shifts subsequent rows).
  const itemsResult = React.useMemo(
    () => buildTimelineItems(entries, firstUnreadMessageId),
    [entries, firstUnreadMessageId],
  );
  const dayGroups = React.useMemo(
    () => buildTimelineDayGroups(itemsResult.items),
    [itemsResult.items],
  );

  const renderItem = React.useCallback(
    (item: TimelineNonDayItem) => {
      switch (item.kind) {
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
              huddleMemberPubkeys={huddleMemberPubkeys}
              huddleMemberPubkeysPending={huddleMemberPubkeysPending}
              isContinuation={item.isContinuation}
              isFollowedByContinuation={item.isFollowedByContinuation}
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
      huddleMemberPubkeys,
      huddleMemberPubkeysPending,
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

  if (useVirtualizer) {
    return (
      <VirtualizedTimelineRows
        dayGroups={dayGroups}
        onAtBottomStateChange={onAtBottomStateChange}
        onStartReached={onStartReached}
        onVirtualizerApiChange={onVirtualizerApiChange}
        onVirtualizerRangeChanged={onVirtualizerRangeChanged}
        onVirtualizerScroll={onVirtualizerScroll}
        onVirtualizerScrollerChange={onVirtualizerScrollerChange}
        renderItem={renderItem}
      />
    );
  }

  return (
    <div className="flex flex-col">
      {dayGroups.map((group) => (
        <section
          className={cn(
            "relative flex flex-col",
            group.headingTimestamp !== null &&
              "before:absolute before:inset-x-0 before:top-4 before:h-px before:bg-border/35 before:content-['']",
          )}
          data-day-label={
            group.headingTimestamp === null
              ? undefined
              : formatDayHeading(group.headingTimestamp)
          }
          data-testid="message-timeline-day-group"
          key={group.key}
        >
          {group.headingTimestamp === null ? null : (
            <DayDivider label={formatDayHeading(group.headingTimestamp)} />
          )}
          {group.items.map((item) => (
            <TimelineRowShell item={item} key={getTimelineItemKey(item)}>
              {renderItem(item)}
            </TimelineRowShell>
          ))}
        </section>
      ))}
    </div>
  );
});

const FIRST_ITEM_INDEX_BASE = 1_000_000;

function timelineItemMessageId(item: TimelineNonDayItem): string | null {
  return item.kind === "message" || item.kind === "system"
    ? item.entry.message.id
    : null;
}

function groupedInternalIndexForItem(
  groupCounts: readonly number[],
  itemIndex: number,
): number {
  let itemCount = 0;

  for (let groupIndex = 0; groupIndex < groupCounts.length; groupIndex += 1) {
    if (itemIndex < itemCount + groupCounts[groupIndex]) {
      return itemIndex + groupIndex + 1;
    }
    itemCount += groupCounts[groupIndex];
  }

  return itemIndex + groupCounts.length;
}

function buildGroupedInternalKeyByIndex(
  firstItemIndex: number,
  dayGroups: readonly TimelineDayGroup[],
): Map<number, string> {
  const keyByIndex = new Map<number, string>();
  let internalIndex = 0;

  for (const group of dayGroups) {
    keyByIndex.set(firstItemIndex + internalIndex, `day-${group.key}`);
    internalIndex += 1;

    for (const item of group.items) {
      keyByIndex.set(firstItemIndex + internalIndex, getTimelineItemKey(item));
      internalIndex += 1;
    }
  }

  return keyByIndex;
}

function buildGroupedInternalData(
  dayGroups: readonly TimelineDayGroup[],
): Array<TimelineNonDayItem | undefined> {
  const data: Array<TimelineNonDayItem | undefined> = [];

  for (const group of dayGroups) {
    data.push(undefined);
    data.push(...group.items);
  }

  return data;
}

type VirtualizedTimelineRowsProps = {
  dayGroups: TimelineDayGroup[];
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onStartReached?: () => void;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
  onVirtualizerScroll?: () => void;
  onVirtualizerScrollerChange?: (element: HTMLDivElement | null) => void;
  renderItem: (item: TimelineNonDayItem) => React.ReactNode;
};

function VirtualizedTimelineRows({
  dayGroups,
  onAtBottomStateChange,
  onStartReached,
  onVirtualizerApiChange,
  onVirtualizerRangeChanged,
  onVirtualizerScroll,
  onVirtualizerScrollerChange,
  renderItem,
}: VirtualizedTimelineRowsProps) {
  const virtuosoRef = React.useRef<GroupedVirtuosoHandle>(null);
  const [scrollerElement, setScrollerElement] =
    React.useState<HTMLDivElement | null>(null);
  const firstItemIndexStateRef = React.useRef<StableFirstItemIndexState>({
    firstItemIndex: FIRST_ITEM_INDEX_BASE,
    keys: [],
  });
  const timelineModel = React.useMemo(() => {
    const items = dayGroups.flatMap((group) => group.items);
    return {
      firstItemIndex: getStableFirstItemIndex(
        firstItemIndexStateRef.current,
        items,
      ),
      flattenedItems: items,
      groupCounts: dayGroups.map((group) => group.items.length),
    };
  }, [dayGroups]);
  const { firstItemIndex, flattenedItems, groupCounts } = timelineModel;
  const groupedInternalKeyByIndex = React.useMemo(
    () => buildGroupedInternalKeyByIndex(firstItemIndex, dayGroups),
    [dayGroups, firstItemIndex],
  );
  const groupedInternalData = React.useMemo(
    () => buildGroupedInternalData(dayGroups),
    [dayGroups],
  );
  const virtualizerModelRef = React.useRef<{
    firstItemIndex: number;
    flattenedCount: number;
    groupCounts: readonly number[];
    messageItemIndexById: Map<string, number>;
  }>({
    firstItemIndex,
    flattenedCount: flattenedItems.length,
    groupCounts,
    messageItemIndexById: new Map(),
  });
  const deferredScrollFrameRef = React.useRef<number | null>(null);
  React.useEffect(
    () => () => {
      if (deferredScrollFrameRef.current !== null) {
        window.cancelAnimationFrame(deferredScrollFrameRef.current);
        deferredScrollFrameRef.current = null;
      }
    },
    [],
  );
  const messageItemIndexById = React.useMemo(() => {
    const byId = new Map<string, number>();
    flattenedItems.forEach((item, index) => {
      const messageId = timelineItemMessageId(item);
      if (messageId) byId.set(messageId, index);
    });
    return byId;
  }, [flattenedItems]);
  React.useLayoutEffect(() => {
    virtualizerModelRef.current = {
      firstItemIndex,
      flattenedCount: flattenedItems.length,
      groupCounts,
      messageItemIndexById,
    };
  }, [
    firstItemIndex,
    flattenedItems.length,
    groupCounts,
    messageItemIndexById,
  ]);

  React.useLayoutEffect(() => {
    if (!onVirtualizerApiChange) return;
    const api: TimelineVirtualizerApi = {
      scrollToBottom(behavior = "auto") {
        virtuosoRef.current?.scrollToIndex({
          align: "end",
          behavior: behavior === "smooth" ? "smooth" : "auto",
          index: "LAST",
        });
      },
      scrollToMessage(messageId, options = {}) {
        const itemIndex = messageItemIndexById.get(messageId);
        if (itemIndex === undefined) return false;
        if (deferredScrollFrameRef.current !== null) {
          window.cancelAnimationFrame(deferredScrollFrameRef.current);
        }
        const requested = {
          count: flattenedItems.length,
          firstItemIndex,
          itemIndex,
          messageId,
        };
        deferredScrollFrameRef.current = window.requestAnimationFrame(() => {
          deferredScrollFrameRef.current = null;
          const current = virtualizerModelRef.current;
          const currentIndex = current.messageItemIndexById.get(messageId);
          if (
            currentIndex !== requested.itemIndex ||
            current.firstItemIndex !== requested.firstItemIndex ||
            current.flattenedCount !== requested.count
          ) {
            return;
          }
          virtuosoRef.current?.scrollToIndex({
            align: "center",
            behavior: options.behavior === "smooth" ? "smooth" : "auto",
            index: groupedInternalIndexForItem(
              current.groupCounts,
              requested.itemIndex,
            ),
          });
        });
        return true;
      },
    };
    onVirtualizerApiChange(api);
    return () => onVirtualizerApiChange(null);
  }, [
    flattenedItems.length,
    firstItemIndex,
    messageItemIndexById,
    onVirtualizerApiChange,
  ]);

  const handleScrollerRef = React.useCallback(
    (ref: HTMLElement | Window | null) => {
      const scroller = ref instanceof HTMLDivElement ? ref : null;
      onVirtualizerScrollerChange?.(scroller);
      setScrollerElement(scroller);
    },
    [onVirtualizerScrollerChange],
  );

  React.useEffect(() => {
    if (!scrollerElement || !onVirtualizerScroll) return;
    scrollerElement.addEventListener("scroll", onVirtualizerScroll, {
      passive: true,
    });
    return () =>
      scrollerElement.removeEventListener("scroll", onVirtualizerScroll);
  }, [onVirtualizerScroll, scrollerElement]);

  const handleRangeChanged = React.useCallback(
    (_range: ListRange) => {
      onVirtualizerRangeChanged?.();
    },
    [onVirtualizerRangeChanged],
  );

  return (
    <GroupedVirtuoso<TimelineNonDayItem | undefined>
      ref={virtuosoRef}
      className="h-full min-h-0 w-full"
      components={virtuosoComponents}
      computeItemKey={(index, item) =>
        item === undefined
          ? (groupedInternalKeyByIndex.get(index) ??
            `timeline-missing-${index}`)
          : getTimelineItemKey(item)
      }
      data={groupedInternalData}
      defaultItemHeight={72}
      firstItemIndex={firstItemIndex}
      followOutput="auto"
      atBottomStateChange={onAtBottomStateChange}
      initialTopMostItemIndex={{ align: "end", index: "LAST" }}
      groupContent={(groupIndex) => {
        const group = dayGroups[groupIndex];
        return (
          <div
            className={cn(
              "relative flex flex-col",
              group.headingTimestamp !== null &&
                "before:absolute before:inset-x-0 before:top-4 before:h-px before:bg-border/35 before:content-['']",
            )}
            data-day-label={
              group.headingTimestamp === null
                ? undefined
                : formatDayHeading(group.headingTimestamp)
            }
            data-testid="message-timeline-day-group"
          >
            {group.headingTimestamp === null ? null : (
              <DayDivider label={formatDayHeading(group.headingTimestamp)} />
            )}
          </div>
        );
      }}
      groupCounts={groupCounts}
      itemContent={(_index, _groupIndex, item) => {
        if (!item) return null;
        return (
          <TimelineRowShell item={item} useContentVisibility={false}>
            {renderItem(item)}
          </TimelineRowShell>
        );
      }}
      rangeChanged={handleRangeChanged}
      scrollerRef={handleScrollerRef}
      startReached={onStartReached ? () => onStartReached() : undefined}
    />
  );
}

const virtuosoComponents: Components<TimelineNonDayItem | undefined> = {
  Scroller: React.forwardRef<
    HTMLDivElement,
    ScrollerProps & { className?: string }
  >(function TimelineVirtuosoScroller({ className, ...props }, ref) {
    return (
      <div
        {...props}
        className={cn(
          "h-full min-h-0 overflow-y-auto overflow-x-hidden overscroll-contain px-2 pt-1 pb-24",
          className,
        )}
        data-buzz-conversation-scroll
        data-testid="message-timeline"
        data-virtuoso-scroller="true"
        ref={ref}
      />
    );
  }),
  List: React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<"div">>(
    function TimelineVirtuosoList({ className, ...props }, ref) {
      return (
        <div {...props} className={cn("flex flex-col", className)} ref={ref} />
      );
    },
  ),
};

function TimelineRowShell({
  children,
  item,
  useContentVisibility = true,
}: {
  children: React.ReactNode;
  item: TimelineNonDayItem;
  useContentVisibility?: boolean;
}) {
  return (
    <div
      className={useContentVisibility ? "timeline-row-cv" : undefined}
      style={useContentVisibility ? timelineRowReserveStyle(item) : undefined}
    >
      {children}
    </div>
  );
}

type StableFirstItemIndexState = {
  firstItemIndex: number;
  keys: string[];
};

function getStableFirstItemIndex(
  state: StableFirstItemIndexState,
  items: TimelineNonDayItem[],
): number {
  const keys = items.map(getTimelineItemKey);
  let firstItemIndex = state.firstItemIndex;

  if (state.keys.length === 0 || keys.length === 0) {
    firstItemIndex = FIRST_ITEM_INDEX_BASE;
  } else {
    const previousFirstKey = state.keys[0];
    const previousFirstIndex = keys.indexOf(previousFirstKey);
    if (previousFirstIndex > 0) {
      firstItemIndex -= previousFirstIndex;
    } else if (previousFirstIndex === -1) {
      firstItemIndex = FIRST_ITEM_INDEX_BASE;
    }
  }

  state.firstItemIndex = firstItemIndex;
  state.keys = keys;
  return firstItemIndex;
}

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
  | "huddleMemberPubkeys"
  | "huddleMemberPubkeysPending"
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
  isContinuation?: boolean;
  isFollowedByContinuation?: boolean;
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
  huddleMemberPubkeys,
  huddleMemberPubkeysPending,
  isContinuation = false,
  isFollowedByContinuation = false,
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
  const canManage = canManageMessageForCurrentUser(
    message,
    currentPubkey,
    profiles,
  );
  const canDelete = canManage && onDelete ? onDelete : undefined;
  const canEdit = canManage && onEdit ? onEdit : undefined;

  if (summary && onReply) {
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
          agentPubkeys={agentPubkeys}
          channelId={channelId}
          highlighted={false}
          hoverBackground={false}
          huddleMemberPubkeys={huddleMemberPubkeys}
          huddleMemberPubkeysPending={huddleMemberPubkeysPending}
          isFollowingThread={
            isFollowingThreadById
              ? isFollowingThreadById(message.id)
              : undefined
          }
          isUnread={isUnread}
          isContinuation={isContinuation}
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
        agentPubkeys={agentPubkeys}
        channelId={channelId}
        highlighted={message.id === highlightedMessageId || isSearchActive}
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
