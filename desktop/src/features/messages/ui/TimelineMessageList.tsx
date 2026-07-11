import * as React from "react";
import { VList } from "virtua";
import type { VListHandle } from "virtua";

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
import { useTimelineRetention } from "./useTimelineRetention";
import { useUpwardPaginationWheel } from "./useUpwardPaginationWheel";

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
  /** Content rendered as the first virtual row before channel history. */
  leadingContent?: React.ReactNode;
  /** The virtualized timeline owns its scroll node when enabled. */
  useVirtualizer?: boolean;
  onStartReached?: () => boolean;
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
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
  leadingContent,
  useVirtualizer = false,
  onStartReached,
  onAtBottomStateChange,
  onVirtualizerApiChange,
  onVirtualizerRangeChanged,
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
        leadingContent={leadingContent}
        onAtBottomStateChange={onAtBottomStateChange}
        onStartReached={onStartReached}
        onVirtualizerApiChange={onVirtualizerApiChange}
        onVirtualizerRangeChanged={onVirtualizerRangeChanged}
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

type VirtualizedTimelineItem =
  | {
      kind: "leading-content";
      content: React.ReactNode;
      hasTopClearance: boolean;
    }
  | { kind: "bottom-spacer" }
  | {
      kind: "timeline-item";
      item: TimelineNonDayItem;
      dayHeading: Pick<TimelineDayGroup, "headingTimestamp"> | null;
      hasTopClearance: boolean;
    };

function timelineItemMessageId(item: TimelineNonDayItem): string | null {
  return item.kind === "message" || item.kind === "system"
    ? item.entry.message.id
    : null;
}

function virtualizedItemKey(item: VirtualizedTimelineItem): string {
  if (item.kind === "bottom-spacer") return "bottom-spacer";
  if (item.kind === "leading-content") return "leading-content";
  return getTimelineItemKey(item.item);
}

function buildVirtualizedItems(
  dayGroups: readonly TimelineDayGroup[],
  leadingContent?: React.ReactNode,
): VirtualizedTimelineItem[] {
  const timelineItems = dayGroups.flatMap((group) =>
    group.items.map((item, index) => ({
      kind: "timeline-item" as const,
      item,
      dayHeading:
        index === 0
          ? {
              headingTimestamp: group.headingTimestamp,
            }
          : null,
      hasTopClearance: !leadingContent && group === dayGroups[0] && index === 0,
    })),
  );

  return [
    ...(leadingContent
      ? [
          {
            kind: "leading-content" as const,
            content: leadingContent,
            hasTopClearance: true,
          },
        ]
      : []),
    ...timelineItems,
    { kind: "bottom-spacer" as const },
  ];
}

type VirtualizedTimelineRowsProps = {
  dayGroups: TimelineDayGroup[];
  leadingContent?: React.ReactNode;
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onStartReached?: () => boolean;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
  onVirtualizerScrollerChange?: (element: HTMLDivElement | null) => void;
  renderItem: (item: TimelineNonDayItem) => React.ReactNode;
};

function didPrependVirtualizedTimeline(
  previousKeys: readonly string[],
  keys: readonly string[],
): boolean {
  const prependedCount = keys.length - previousKeys.length;
  // Virtua shifts its positional size cache by the length delta, so enable
  // `shift` only when every previous slot is the exact suffix it expects.
  return (
    prependedCount > 0 &&
    previousKeys.every((key, index) => key === keys[index + prependedCount])
  );
}

function VirtualizedTimelineRows({
  dayGroups,
  leadingContent,
  onAtBottomStateChange,
  onStartReached,
  onVirtualizerApiChange,
  onVirtualizerRangeChanged,
  onVirtualizerScrollerChange,
  renderItem,
}: VirtualizedTimelineRowsProps) {
  const listRef = React.useRef<VListHandle>(null);
  const hostRef = React.useRef<HTMLDivElement>(null);
  const itemsLengthRef = React.useRef(0);
  const messageItemIndexByIdRef = React.useRef<ReadonlyMap<string, number>>(
    new Map(),
  );
  const [offscreenBufferSize, setOffscreenBufferSize] = React.useState(() =>
    typeof window === "undefined" ? 1_000 : window.innerHeight,
  );
  const initialPositionFrameRef = React.useRef<number | null>(null);
  const hasInitialPositionedRef = React.useRef(false);
  const items = React.useMemo(
    () => buildVirtualizedItems(dayGroups, leadingContent),
    [dayGroups, leadingContent],
  );
  const keys = React.useMemo(() => items.map(virtualizedItemKey), [items]);
  itemsLengthRef.current = items.length;
  const previousKeysRef = React.useRef<readonly string[]>([]);
  const prependAnchorRef = React.useRef<{
    messageId: string;
    top: number;
  } | null>(null);
  const prependWatcherFrameRef = React.useRef<number | null>(null);
  const [prependShiftEpoch, clearPrependShift] = React.useReducer(
    (version: number) => version + 1,
    0,
  );
  // Virtua's `shift` is a one-render instruction, not a persistent mode. If it
  // stays true after a prepend, later measurement changes can keep anchoring
  // from the end and leave a stale blank range until the next scroll event.
  const isPrepend = React.useMemo(() => {
    void prependShiftEpoch;
    return didPrependVirtualizedTimeline(previousKeysRef.current, keys);
  }, [keys, prependShiftEpoch]);

  const retirePrependAnchor = React.useCallback(() => {
    if (prependWatcherFrameRef.current !== null) {
      cancelAnimationFrame(prependWatcherFrameRef.current);
    }
    prependWatcherFrameRef.current = null;
    prependAnchorRef.current = null;
  }, []);
  const { arm: armUpwardMomentum, clear: clearUpwardMomentum } =
    useUpwardPaginationWheel(hostRef, retirePrependAnchor);

  const capturePrependAnchor = React.useCallback(() => {
    // Keep the pending capture current while the fetch is in flight. Once the
    // prepend commits and the watcher starts, its baseline is frozen.
    if (prependWatcherFrameRef.current !== null) return;
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    const scrollerTop = scroller.getBoundingClientRect().top;
    const row = Array.from(
      scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
    ).find((candidate) => candidate.getBoundingClientRect().top >= scrollerTop);
    const messageId = row?.dataset.messageId;
    if (!row || !messageId) return;
    prependAnchorRef.current = {
      messageId,
      top: row.getBoundingClientRect().top - scrollerTop,
    };
  }, []);

  React.useLayoutEffect(() => {
    if (!isPrepend || !prependAnchorRef.current) return;
    // Virtua's shift mode correctly absorbs most prepended measurements, but
    // estimated offsets can misclassify late row growth deep in history. Keep
    // the semantic row identity as a short-lived, deviation-gated backstop.
    // This watcher deliberately survives a temporary row unmount and uses only
    // relative correction so Virtua remains the primary scroll owner.
    if (prependWatcherFrameRef.current !== null) {
      cancelAnimationFrame(prependWatcherFrameRef.current);
    }
    const anchor = prependAnchorRef.current;
    const scroller = hostRef.current?.firstElementChild;
    if (scroller instanceof HTMLDivElement) {
      const row = Array.from(
        scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
      ).find((candidate) => candidate.dataset.messageId === anchor.messageId);
      if (row) {
        const top =
          row.getBoundingClientRect().top -
          scroller.getBoundingClientRect().top;
        const delta = top - anchor.top;
        // Close the commit-frame displacement before paint. The watcher below
        // remains responsible for late measurements and temporary unmounts.
        if (Math.abs(delta) > 4) scroller.scrollBy({ top: delta });
      }
    }
    const deadline = performance.now() + 3_000;
    let previousScrollTop: number | null = null;
    let settledFrames = 0;

    const watch = () => {
      const scroller = hostRef.current?.firstElementChild;
      if (!(scroller instanceof HTMLDivElement)) {
        prependWatcherFrameRef.current = null;
        prependAnchorRef.current = null;
        return;
      }
      const atBottom =
        scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop <=
        32;
      const row = Array.from(
        scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
      ).find((candidate) => candidate.dataset.messageId === anchor.messageId);
      const top = row
        ? row.getBoundingClientRect().top - scroller.getBoundingClientRect().top
        : null;
      const scrollTop = scroller.scrollTop;
      settledFrames =
        previousScrollTop !== null &&
        Math.abs(scrollTop - previousScrollTop) < 0.5
          ? settledFrames + 1
          : 0;
      previousScrollTop = scrollTop;

      if (row && top !== null && settledFrames >= 2) {
        const delta = top - anchor.top;
        if (Math.abs(delta) > 4) {
          scroller.scrollBy({ top: delta });
          settledFrames = 0;
          previousScrollTop = null;
        }
      }

      const retired =
        performance.now() >= deadline ||
        atBottom ||
        (top !== null && top > scroller.clientHeight * 2);
      if (retired) {
        retirePrependAnchor();
        return;
      }
      prependWatcherFrameRef.current = requestAnimationFrame(watch);
    };
    prependWatcherFrameRef.current = requestAnimationFrame(watch);
    clearUpwardMomentum();
  }, [clearUpwardMomentum, isPrepend, retirePrependAnchor]);

  React.useEffect(() => () => retirePrependAnchor(), [retirePrependAnchor]);

  React.useLayoutEffect(() => {
    previousKeysRef.current = keys;
    if (isPrepend) {
      clearPrependShift();
    }
    if (!hasInitialPositionedRef.current && items.length > 0) {
      hasInitialPositionedRef.current = true;
      const scrollToSettledBottom = () => {
        listRef.current?.scrollToIndex(items.length - 1, { align: "end" });
      };
      scrollToSettledBottom();
      initialPositionFrameRef.current = requestAnimationFrame(() => {
        initialPositionFrameRef.current = null;
        scrollToSettledBottom();
      });
    }
  }, [isPrepend, items.length, keys]);

  React.useEffect(
    () => () => {
      if (initialPositionFrameRef.current !== null) {
        cancelAnimationFrame(initialPositionFrameRef.current);
      }
    },
    [],
  );

  const messageItemIndexById = React.useMemo(() => {
    const byId = new Map<string, number>();
    items.forEach((item, index) => {
      if (item.kind !== "timeline-item") return;
      const messageId = timelineItemMessageId(item.item);
      if (messageId) byId.set(messageId, index);
    });
    return byId;
  }, [items]);
  messageItemIndexByIdRef.current = messageItemIndexById;

  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    const element = scroller instanceof HTMLDivElement ? scroller : null;
    if (element) {
      element.dataset.buzzConversationScroll = "true";
      element.dataset.testid = "message-timeline";
    }
    onVirtualizerScrollerChange?.(element);
    return () => onVirtualizerScrollerChange?.(null);
  }, [onVirtualizerScrollerChange]);

  React.useLayoutEffect(() => {
    if (!onVirtualizerApiChange) return;
    const api: TimelineVirtualizerApi = {
      scrollToBottom() {
        retirePrependAnchor();
        const lastIndex = itemsLengthRef.current - 1;
        if (lastIndex >= 0) {
          listRef.current?.scrollToIndex(lastIndex, { align: "end" });
        }
      },
      scrollToMessage(messageId) {
        retirePrependAnchor();
        const index = messageItemIndexByIdRef.current.get(messageId);
        if (index === undefined) return false;
        listRef.current?.scrollToIndex(index, { align: "center" });
        return true;
      },
    };
    onVirtualizerApiChange(api);
    return () => onVirtualizerApiChange(null);
  }, [onVirtualizerApiChange, retirePrependAnchor]);

  React.useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const updateBufferSize = () => {
      // Measure rows three viewports ahead of the reader. Virtua deliberately
      // hides each newly mounted row until its first ResizeObserver result; a
      // one-viewport lead can be consumed by WebKit trackpad momentum before
      // that result commits, producing a first-pass-only blank flash. The
      // measured size is cached, which is why revisiting the same range is
      // already stable.
      setOffscreenBufferSize(host.clientHeight * 3);
    };
    updateBufferSize();
    const resizeObserver = new ResizeObserver(updateBufferSize);
    resizeObserver.observe(host);
    return () => resizeObserver.disconnect();
  }, []);

  const { retainedIndices, onScrollEnd: handleScrollEnd } =
    useTimelineRetention(keys, listRef, isPrepend);

  const handleScroll = React.useCallback(
    (offset: number) => {
      const list = listRef.current;
      const scroller = hostRef.current?.firstElementChild;
      if (!list || !(scroller instanceof HTMLDivElement)) return;
      onVirtualizerRangeChanged?.();
      onAtBottomStateChange?.(
        list.scrollSize - list.viewportSize - offset <= 32,
      );
      if (
        prependAnchorRef.current !== null ||
        offset <= 200 ||
        prependWatcherFrameRef.current === null
      ) {
        capturePrependAnchor();
      }
      if (offset <= 200) {
        // Layout scrolls near the top must not poison the reader's next input.
        armUpwardMomentum(onStartReached?.() ?? false);
      }
    },
    [
      armUpwardMomentum,
      capturePrependAnchor,
      onAtBottomStateChange,
      onStartReached,
      onVirtualizerRangeChanged,
    ],
  );

  return (
    <div className="h-full min-h-0 w-full" ref={hostRef}>
      <VList
        ref={listRef}
        className="h-full min-h-0 w-full overflow-y-auto overflow-x-hidden overscroll-contain px-2"
        data={items}
        bufferSize={offscreenBufferSize}
        keepMounted={retainedIndices}
        style={{ overflowAnchor: "none" }}
        shift={isPrepend}
        onScroll={handleScroll}
        onScrollEnd={handleScrollEnd}
      >
        {(item) => {
          if (item.kind === "bottom-spacer") {
            return (
              <div
                aria-hidden
                className="h-[var(--composer-overlay-height,6rem)]"
                key={virtualizedItemKey(item)}
              />
            );
          }
          if (item.kind === "leading-content") {
            return (
              <div key={virtualizedItemKey(item)}>
                {item.hasTopClearance ? (
                  <div
                    aria-hidden
                    className="h-[var(--channel-top-chrome-height,4.5rem)]"
                  />
                ) : null}
                {item.content}
              </div>
            );
          }
          const { dayHeading } = item;
          return (
            <TimelineRowShell
              dayLabel={
                dayHeading?.headingTimestamp === null || !dayHeading
                  ? undefined
                  : formatDayHeading(dayHeading.headingTimestamp)
              }
              item={item.item}
              key={virtualizedItemKey(item)}
              useContentVisibility={false}
            >
              {item.hasTopClearance ? (
                <div
                  aria-hidden
                  className="h-[var(--channel-top-chrome-height,4.5rem)]"
                />
              ) : null}
              {dayHeading?.headingTimestamp !== null && dayHeading ? (
                <DayDivider
                  label={formatDayHeading(dayHeading.headingTimestamp)}
                />
              ) : null}
              {renderItem(item.item)}
            </TimelineRowShell>
          );
        }}
      </VList>
    </div>
  );
}

function TimelineRowShell({
  children,
  dayLabel,
  item,
  useContentVisibility = true,
}: {
  children: React.ReactNode;
  dayLabel?: string;
  item: TimelineNonDayItem;
  useContentVisibility?: boolean;
}) {
  return (
    <div
      className={cn(
        dayLabel &&
          "relative flex flex-col before:absolute before:inset-x-0 before:top-4 before:h-px before:bg-border/35 before:content-['']",
        useContentVisibility && "timeline-row-cv",
      )}
      data-day-label={dayLabel}
      data-testid={dayLabel ? "message-timeline-day-group" : undefined}
      style={useContentVisibility ? timelineRowReserveStyle(item) : undefined}
    >
      {children}
    </div>
  );
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
