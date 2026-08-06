import * as React from "react";
import { VList } from "virtua";
import type { VListHandle } from "virtua";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import {
  buildTimelineDayGroups,
  buildTimelineItems,
  getTimelineItemKey,
  type TimelineDayGroup,
  type TimelineNonDayItem,
} from "@/features/messages/lib/timelineItems";
import {
  buildVirtualizedItems,
  didPrependVirtualizedTimeline,
  estimateVirtualizedTimelineItemHeight,
  type VirtualizedTimelineItem,
  virtualizedItemKey,
} from "@/features/messages/lib/virtualizedTimelineItems";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import { buildVideoReviewContextsByMessageId } from "@/features/messages/lib/videoReviewContext";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { DayDivider } from "./DayDivider";
import { MessageRowItem, SystemRow } from "./TimelineMessageRow";
import { TimelineRowShell } from "./TimelineRowShell";
import { UnreadDivider } from "./UnreadDivider";
import { useTimelineRetention } from "./useTimelineRetention";
import { useUpwardPaginationWheel } from "./useUpwardPaginationWheel";
import { useVirtualizedBottomSettle } from "./useVirtualizedBottomSettle";

export type TimelineVirtualizerApi = {
  cancelBottomIntent: () => void;
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  settleAtBottom: () => void;
  scrollToMessage: (
    messageId: string,
    options?: { behavior?: ScrollBehavior },
  ) => boolean;
};

type TimelineMessageListProps = {
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
  entranceMessageId?: string | null;
  onEntranceMessageComplete?: (messageId: string) => void;
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
  onOpenThread?: (message: TimelineMessage) => void;
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
  ownerProfiles?: UserProfileLookup;
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
  /** Hide date boundaries for a huddle's live transcript. */
  hideDayDividers?: boolean;
  /** Show speaker identity on every row instead of grouping consecutive messages. */
  alwaysShowMessageIdentity?: boolean;
  /** Hide agent access-policy badges in the purpose-built Huddle chat. */
  hideAgentAccessBadges?: boolean;
  /**
   * True when the loaded window provably starts at the channel's beginning.
   * Proves the oldest loaded day's boundary so its divider may render.
   */
  historyExhausted?: boolean;
  /** The virtualized timeline owns its scroll node when enabled. */
  useVirtualizer?: boolean;
  onStartReached?: () => boolean;
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
  onVirtualizerScrollerChange?: (element: HTMLDivElement | null) => void;
};

export const TimelineMessageList = React.memo(function TimelineMessageList({
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
  entranceMessageId = null,
  onEntranceMessageComplete,
  messageFooters,
  mainEntries,
  threadSummaries,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onMarkRead,
  onReply,
  onOpenThread,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  profiles,
  ownerProfiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  threadUnreadCounts,
  unfollowThreadById,
  leadingContent,
  historyExhausted = false,
  hideDayDividers = false,
  alwaysShowMessageIdentity = false,
  hideAgentAccessBadges = false,
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
  // Contexts are memoized per message id so MessageRow/Markdown memo
  // comparisons hold across unrelated timeline re-renders (typing
  // indicators, presence updates) — a fresh context object per render would
  // defeat the memo and re-render every video message on every pass.
  const videoReviewContextById = React.useMemo(() => {
    return buildVideoReviewContextsByMessageId({
      channelId,
      channelName,
      channelType,
      isSendingVideoReviewComment,
      messages,
      onSendVideoReviewComment,
      onToggleReaction,
      profiles,
    });
  }, [
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messages,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
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
              ownerProfiles={ownerProfiles}
            />
          );
        case "system-group":
          return (
            <SystemRow
              currentPubkey={currentPubkey}
              entries={item.entries}
              footer={item.entries.map(
                (entry) => messageFooters?.[entry.message.id] ?? null,
              )}
              onToggleReaction={onToggleReaction}
              profiles={profiles}
              ownerProfiles={ownerProfiles}
            />
          );
        case "message":
          return (
            <MessageRowItem
              channelId={channelId}
              currentPubkey={currentPubkey}
              entry={item.entry}
              followThreadById={followThreadById}
              footer={messageFooters?.[item.entry.message.id] ?? null}
              highlightedMessageId={highlightedMessageId}
              huddleMemberPubkeys={huddleMemberPubkeys}
              huddleMemberPubkeysPending={huddleMemberPubkeysPending}
              hideAgentAccessBadges={hideAgentAccessBadges}
              isContinuation={
                alwaysShowMessageIdentity ? false : item.isContinuation
              }
              isFollowedByContinuation={
                alwaysShowMessageIdentity
                  ? false
                  : item.isFollowedByContinuation
              }
              isFollowingThreadById={isFollowingThreadById}
              isUnread={isMessageUnreadById?.(item.entry.message.id)}
              playEntrance={item.entry.message.id === entranceMessageId}
              onEntranceComplete={onEntranceMessageComplete}
              onDelete={onDelete}
              onEdit={onEdit}
              onMarkRead={onMarkRead}
              onMarkUnread={onMarkUnread}
              onReply={onReply}
              onOpenThread={onOpenThread}
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
      channelId,
      alwaysShowMessageIdentity,
      currentPubkey,
      followThreadById,
      highlightedMessageId,
      huddleMemberPubkeys,
      huddleMemberPubkeysPending,
      hideAgentAccessBadges,
      isFollowingThreadById,
      isMessageUnreadById,
      entranceMessageId,
      onEntranceMessageComplete,
      messageFooters,
      onDelete,
      onEdit,
      onMarkRead,
      onMarkUnread,
      onReply,
      onOpenThread,
      onToggleReaction,
      profiles,
      ownerProfiles,
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
        historyExhausted={historyExhausted}
        hideDayDividers={hideDayDividers}
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
            !hideDayDividers &&
              group.headingTimestamp !== null &&
              "before:absolute before:inset-x-0 before:top-1/2 before:h-px before:-translate-y-1/2 before:bg-border/35 before:content-['']",
          )}
          data-day-label={
            group.headingTimestamp === null
              ? undefined
              : formatDayHeading(group.headingTimestamp)
          }
          data-testid="message-timeline-day-group"
          key={group.key}
        >
          {hideDayDividers || group.headingTimestamp === null ? null : (
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

function timelineItemMessageIds(item: TimelineNonDayItem): string[] {
  if (item.kind === "system-group") {
    return item.entries.map((entry) => entry.message.id);
  }
  return item.kind === "message" || item.kind === "system"
    ? [item.entry.message.id]
    : [];
}

type VirtualizedTimelineRowsProps = {
  dayGroups: TimelineDayGroup[];
  historyExhausted: boolean;
  hideDayDividers: boolean;
  leadingContent?: React.ReactNode;
  onAtBottomStateChange?: (atBottom: boolean) => void;
  onStartReached?: () => boolean;
  onVirtualizerApiChange?: (api: TimelineVirtualizerApi | null) => void;
  onVirtualizerRangeChanged?: () => void;
  onVirtualizerScrollerChange?: (element: HTMLDivElement | null) => void;
  renderItem: (item: TimelineNonDayItem) => React.ReactNode;
};

type VirtualizedTimelineItemShellProps = {
  children: React.ReactNode;
  index: number;
  ref?: React.LegacyRef<HTMLDivElement>;
  style: React.CSSProperties;
};

const PreserveVirtualizedItemVisibilityContext = React.createContext(false);

function VirtualizedTimelineItemShell({
  children,
  ref,
  style,
}: VirtualizedTimelineItemShellProps) {
  const preserveVisibility = React.useContext(
    PreserveVirtualizedItemVisibilityContext,
  );
  return (
    <div
      ref={ref}
      style={preserveVisibility ? style : { ...style, visibility: undefined }}
    >
      {children}
    </div>
  );
}

function VirtualizedTimelineRows({
  dayGroups,
  historyExhausted,
  hideDayDividers,
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
  const hasInitialPositionedRef = React.useRef(false);
  const pinnedDayLabelRef = React.useRef<HTMLDivElement>(null);
  const pinnedDayTranslateYRef = React.useRef(0);
  const estimateCallCountRef = React.useRef(0);
  const estimateItemSize = React.useCallback(
    (item: VirtualizedTimelineItem) => {
      estimateCallCountRef.current += 1;
      const scroller = hostRef.current?.firstElementChild;
      if (scroller instanceof HTMLDivElement) {
        scroller.dataset.virtuaEstimateCallCount = String(
          estimateCallCountRef.current,
        );
      }
      return estimateVirtualizedTimelineItemHeight(item);
    },
    [],
  );
  const items = React.useMemo(
    () =>
      buildVirtualizedItems(
        dayGroups,
        leadingContent,
        historyExhausted,
        !hideDayDividers,
      ),
    [dayGroups, hideDayDividers, historyExhausted, leadingContent],
  );
  const keys = React.useMemo(() => items.map(virtualizedItemKey), [items]);
  const dayDividerItems = React.useMemo(
    () =>
      items.flatMap((item, index) =>
        item.kind === "day-divider" ? [{ index, item }] : [],
      ),
    [items],
  );
  const [pinnedDay, setPinnedDay] = React.useState<{
    label: string | null;
    incomingLabel: string | null;
  }>({ label: null, incomingLabel: null });
  itemsLengthRef.current = items.length;
  const previousKeysRef = React.useRef<readonly string[]>([]);
  const [prependShiftEpoch, clearPrependShift] = React.useReducer(
    (version: number) => version + 1,
    0,
  );
  const { cancel: cancelBottomSettle, settle: settleAtBottom } =
    useVirtualizedBottomSettle(hostRef, listRef, itemsLengthRef);
  const { arm: armUpwardMomentum } = useUpwardPaginationWheel(
    hostRef,
    cancelBottomSettle,
  );

  const updatePinnedDayLabel = React.useCallback(
    (offset: number) => {
      const list = listRef.current;
      const scroller = hostRef.current?.firstElementChild;
      const pinnedLabel = pinnedDayLabelRef.current;
      if (!list || !(scroller instanceof HTMLDivElement) || !pinnedLabel) {
        return;
      }

      const pinnedTop =
        pinnedLabel.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top -
        pinnedDayTranslateYRef.current;
      const [pinnedPill, incomingPinnedPill] =
        pinnedLabel.querySelectorAll<HTMLParagraphElement>("p");
      const pinnedPillHeight = pinnedPill?.offsetHeight ?? 0;
      if (pinnedPillHeight === 0) return;
      const renderedDividerPillTop = (
        divider: (typeof dayDividerItems)[number],
      ) => {
        const label = formatDayHeading(divider.item.headingTimestamp);
        const source = [
          ...scroller.querySelectorAll<HTMLElement>(
            '[data-testid="message-timeline-day-divider"]',
          ),
        ].find((element) => element.dataset.dayLabel === label);
        const pill = source?.querySelector<HTMLElement>("p");
        return pill
          ? pill.getBoundingClientRect().top -
              scroller.getBoundingClientRect().top
          : null;
      };
      const sourcePills = [
        ...scroller.querySelectorAll<HTMLElement>(
          '[data-testid="message-timeline-day-divider"] p',
        ),
      ];
      // Source dividers are normally visible in the feed. Only hide the one
      // that physically overlaps the floating chip at the handoff point.
      for (const pill of sourcePills) {
        pill.style.removeProperty("visibility");
      }

      let activeDividerIndex = -1;
      for (const [index, divider] of dayDividerItems.entries()) {
        if (list.getItemOffset(divider.index) > offset + pinnedTop) break;
        activeDividerIndex = index;
      }
      const candidateDivider = dayDividerItems[activeDividerIndex];
      // Retain the previous date while the next in-flow divider is still
      // above the sticky slot. This avoids changing the label before the
      // moving chip reaches its handoff point.
      if (
        activeDividerIndex > 0 &&
        candidateDivider &&
        (renderedDividerPillTop(candidateDivider) ?? -Infinity) > pinnedTop
      ) {
        activeDividerIndex -= 1;
      }
      const activeDivider = dayDividerItems[activeDividerIndex];
      const nextDivider = dayDividerItems[activeDividerIndex + 1];
      const nextDividerTop = nextDivider
        ? (renderedDividerPillTop(nextDivider) ??
          list.getItemOffset(nextDivider.index) - offset)
        : null;
      const nextTranslateY =
        nextDividerTop === null
          ? 0
          : Math.max(
              -pinnedPillHeight,
              Math.min(0, nextDividerTop - pinnedTop - pinnedPillHeight),
            );
      if (pinnedDayTranslateYRef.current !== nextTranslateY) {
        pinnedDayTranslateYRef.current = nextTranslateY;
        pinnedLabel.style.transform = `translateY(${nextTranslateY}px)`;
      }
      const nextLabel = activeDivider
        ? formatDayHeading(activeDivider.item.headingTimestamp)
        : null;
      const incomingLabel =
        nextDivider && nextTranslateY < 0
          ? formatDayHeading(nextDivider.item.headingTimestamp)
          : null;
      const activeSourcePill = sourcePills.find(
        (pill) => pill.parentElement?.dataset.dayLabel === nextLabel,
      );
      if (activeSourcePill) {
        const sourceTop =
          activeSourcePill.getBoundingClientRect().top -
          scroller.getBoundingClientRect().top;
        const overlayTop = pinnedTop;
        const sourceBottom = sourceTop + activeSourcePill.offsetHeight;
        const overlayBottom = overlayTop + pinnedPillHeight;
        if (sourceBottom > overlayTop && sourceTop < overlayBottom) {
          activeSourcePill.style.visibility = "hidden";
        }
      }
      const incomingSourcePill = sourcePills.find(
        (pill) => pill.parentElement?.dataset.dayLabel === incomingLabel,
      );
      if (incomingSourcePill) {
        incomingSourcePill.style.visibility = "hidden";
      }
      if (pinnedPill) {
        pinnedPill.textContent = nextLabel ?? "";
        pinnedPill.style.visibility = nextLabel ? "visible" : "hidden";
      }
      if (incomingPinnedPill) {
        incomingPinnedPill.textContent = incomingLabel ?? "";
        incomingPinnedPill.style.visibility = incomingLabel
          ? "visible"
          : "hidden";
      }
      setPinnedDay((current) =>
        current.label === nextLabel && current.incomingLabel === incomingLabel
          ? current
          : { label: nextLabel, incomingLabel },
      );
    },
    [dayDividerItems],
  );

  React.useEffect(
    () => () => {
      cancelBottomSettle();
    },
    [cancelBottomSettle],
  );

  const isPrepend = React.useMemo(() => {
    void prependShiftEpoch;
    return didPrependVirtualizedTimeline(previousKeysRef.current, keys);
  }, [keys, prependShiftEpoch]);

  React.useLayoutEffect(() => {
    previousKeysRef.current = keys;
    if (isPrepend) {
      clearPrependShift();
    }
    if (!hasInitialPositionedRef.current && items.length > 0) {
      hasInitialPositionedRef.current = true;
      settleAtBottom();
    }
  }, [isPrepend, items.length, keys, settleAtBottom]);

  const messageItemIndexById = React.useMemo(() => {
    const byId = new Map<string, number>();
    items.forEach((item, index) => {
      if (item.kind !== "timeline-item") return;
      for (const messageId of timelineItemMessageIds(item.item)) {
        byId.set(messageId, index);
      }
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
      element.dataset.virtuaEstimateCallCount = String(
        estimateCallCountRef.current,
      );
    }
    onVirtualizerScrollerChange?.(element);
    return () => onVirtualizerScrollerChange?.(null);
  }, [onVirtualizerScrollerChange]);

  React.useLayoutEffect(() => {
    updatePinnedDayLabel(listRef.current?.scrollOffset ?? 0);
  }, [updatePinnedDayLabel]);

  React.useLayoutEffect(() => {
    if (!onVirtualizerApiChange) return;
    const api: TimelineVirtualizerApi = {
      cancelBottomIntent: cancelBottomSettle,
      scrollToBottom() {
        settleAtBottom();
      },
      settleAtBottom,
      scrollToMessage(messageId) {
        cancelBottomSettle();
        const index = messageItemIndexByIdRef.current.get(messageId);
        if (index === undefined) return false;
        listRef.current?.scrollToIndex(index, { align: "center" });
        return true;
      },
    };
    onVirtualizerApiChange(api);
    return () => onVirtualizerApiChange(null);
  }, [cancelBottomSettle, onVirtualizerApiChange, settleAtBottom]);

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
      const distanceFromBottom = list.scrollSize - list.viewportSize - offset;
      // Do not infer reader intent from an intermediate virtualizer offset.
      // Initial channel positioning deliberately chases the floor while rows
      // are measured; those measurements can briefly report a large gap and
      // emit `onScroll` without any user input. Cancelling here strands the
      // channel above its newest message. The settle hook's wheel, pointer,
      // touch, and key listeners are the authoritative user-interaction gate.
      onAtBottomStateChange?.(distanceFromBottom <= 32);
      updatePinnedDayLabel(offset);
      if (offset <= 200) {
        // Layout scrolls near the top must not poison the reader's next input.
        armUpwardMomentum(onStartReached?.() ?? false);
      }
    },
    [
      armUpwardMomentum,
      onAtBottomStateChange,
      onStartReached,
      onVirtualizerRangeChanged,
      updatePinnedDayLabel,
    ],
  );

  return (
    <div className="relative h-full min-h-0 w-full" ref={hostRef}>
      <PreserveVirtualizedItemVisibilityContext value={isPrepend}>
        <VList
          ref={listRef}
          className="h-full min-h-0 w-full overflow-y-auto overflow-x-hidden overscroll-contain px-2 pt-[var(--channel-top-chrome-height,4.5rem)]"
          data={items}
          item={VirtualizedTimelineItemShell}
          itemSize={estimateItemSize}
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
              return <div key={virtualizedItemKey(item)}>{item.content}</div>;
            }
            if (item.kind === "day-divider") {
              const dayLabel = formatDayHeading(item.headingTimestamp);
              return (
                <div
                  className="relative flex flex-col before:absolute before:inset-x-0 before:top-1/2 before:h-px before:-translate-y-1/2 before:bg-border/35 before:content-['']"
                  data-day-label={dayLabel}
                  data-testid="message-timeline-day-group"
                  key={virtualizedItemKey(item)}
                >
                  <DayDivider label={dayLabel} sticky={false} />
                </div>
              );
            }
            return (
              <TimelineRowShell
                item={item.item}
                key={virtualizedItemKey(item)}
                useContentVisibility={false}
              >
                {renderItem(item.item)}
              </TimelineRowShell>
            );
          }}
        </VList>
      </PreserveVirtualizedItemVisibilityContext>
      <div
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-x-0 z-20",
          channelChrome.stickyTimelineTop,
          pinnedDay.label || pinnedDay.incomingLabel
            ? "opacity-100"
            : "opacity-0",
        )}
        data-day-label={pinnedDay.label ?? undefined}
        data-testid="message-timeline-sticky-day-divider"
      >
        <div className="invisible flex justify-center">
          <DayDivider label={pinnedDay.label ?? ""} sticky={false} testId="" />
        </div>
        <div
          className="absolute inset-x-0 top-0 flex flex-col"
          data-testid="message-timeline-sticky-day-divider-content"
          ref={pinnedDayLabelRef}
        >
          <DayDivider label={pinnedDay.label ?? ""} sticky={false} testId="" />
          <DayDivider
            label={pinnedDay.incomingLabel ?? ""}
            sticky={false}
            testId=""
          />
        </div>
      </div>
    </div>
  );
}
