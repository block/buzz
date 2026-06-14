import * as React from "react";
import {
  Virtuoso,
  type FollowOutput,
  type VirtuosoHandle,
} from "react-virtuoso";

import {
  formatDayHeading,
  isSameDay,
} from "@/features/messages/lib/dateFormatters";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { DayDivider } from "./DayDivider";
import {
  buildReviewCommentsByRootId,
  buildVideoReviewContextById,
  renderTimelineMessageEntry,
  type TimelineMessageListProps,
} from "./TimelineMessageList";

type TimelineEntry =
  | {
      key: string;
      type: "day";
      label: string;
    }
  | {
      key: string;
      type: "message";
      message: TimelineMessage;
      summary: ReturnType<typeof buildMainTimelineEntries>[number]["summary"];
    };

type VirtualizedTimelineMessageListProps = TimelineMessageListProps & {
  atBottomStateChange?: (atBottom: boolean) => void;
  bottomFooterClassName?: string;
  followOutput?: FollowOutput;
  hasOlderMessages: boolean;
  isFetchingOlder: boolean;
  onStartReached?: () => void;
  scrollerRef?: (element: HTMLDivElement | null) => void;
  topHeader?: React.ReactNode;
  virtuosoRef?: React.RefObject<VirtuosoHandle | null>;
};

const FIRST_ITEM_INDEX_BASE = 1_000_000;

const TimelineList = React.forwardRef<
  HTMLDivElement,
  React.ComponentProps<"div">
>(function TimelineList({ children, style, ...props }, ref) {
  return (
    <div {...props} className="flex flex-col gap-2" ref={ref} style={style}>
      {children}
    </div>
  );
});
TimelineList.displayName = "VirtualizedTimelineList";

function buildTimelineItems(messages: TimelineMessage[]): TimelineEntry[] {
  const entries = buildMainTimelineEntries(messages);
  const items: TimelineEntry[] = [];

  for (let index = 0; index < entries.length; index += 1) {
    const { message, summary } = entries[index];
    const prev = index > 0 ? entries[index - 1]?.message : null;

    if (!prev || !isSameDay(prev.createdAt, message.createdAt)) {
      items.push({
        key: `day-${message.createdAt}`,
        label: formatDayHeading(message.createdAt),
        type: "day",
      });
    }

    items.push({
      key: message.renderKey ?? message.id,
      message,
      summary,
      type: "message",
    });
  }

  return items;
}

export const VirtualizedTimelineMessageList = React.memo(
  function VirtualizedTimelineMessageList({
    agentPubkeys,
    atBottomStateChange,
    bottomFooterClassName,
    channelId,
    channelName,
    channelType,
    currentPubkey,
    followOutput = false,
    followThreadById,
    hasOlderMessages,
    highlightedMessageId = null,
    isFetchingOlder,
    isFollowingThreadById,
    messageFooters,
    messages,
    onDelete,
    onEdit,
    onMarkUnread,
    onReply,
    onStartReached,
    isSendingVideoReviewComment = false,
    onSendVideoReviewComment,
    onToggleReaction,
    personaLookup,
    profiles,
    scrollerRef,
    searchActiveMessageId = null,
    searchMatchingMessageIds,
    searchQuery,
    topHeader,
    unfollowThreadById,
    virtuosoRef,
  }: VirtualizedTimelineMessageListProps) {
    const items = React.useMemo(() => buildTimelineItems(messages), [messages]);
    const reviewCommentsByRootId = React.useMemo(
      () => buildReviewCommentsByRootId(messages),
      [messages],
    );
    const videoReviewContextById = React.useMemo(
      () =>
        buildVideoReviewContextById({
          channelId,
          channelName,
          channelType,
          isSendingVideoReviewComment,
          messages,
          onSendVideoReviewComment,
          onToggleReaction,
          profiles,
          reviewCommentsByRootId,
        }),
      [
        channelId,
        channelName,
        channelType,
        isSendingVideoReviewComment,
        messages,
        onSendVideoReviewComment,
        onToggleReaction,
        profiles,
        reviewCommentsByRootId,
      ],
    );

    const firstItemIndexStateRef = React.useRef({
      anchorIndex: -1,
      anchorKey: null as string | null,
      firstItemIndex: FIRST_ITEM_INDEX_BASE,
      items: [] as readonly TimelineEntry[],
    });

    const firstItemIndex = React.useMemo(() => {
      const state = firstItemIndexStateRef.current;
      const previousItems = state.items;

      if (items.length === 0) {
        state.anchorIndex = -1;
        state.anchorKey = null;
        state.firstItemIndex = FIRST_ITEM_INDEX_BASE;
        state.items = items;
        return state.firstItemIndex;
      }

      const anchorEntryIndex = items.findIndex(
        (item) => item.type === "message",
      );
      const anchorKey =
        anchorEntryIndex >= 0 ? (items[anchorEntryIndex]?.key ?? null) : null;

      if (previousItems !== items) {
        if (state.anchorKey) {
          const nextAnchorIndex = items.findIndex(
            (item) => item.key === state.anchorKey,
          );
          if (nextAnchorIndex >= 0 && state.anchorIndex >= 0) {
            state.firstItemIndex -= nextAnchorIndex - state.anchorIndex;
          } else {
            state.firstItemIndex = FIRST_ITEM_INDEX_BASE;
          }
        }

        state.anchorIndex = anchorEntryIndex;
        state.anchorKey = anchorKey;
        state.items = items;
      }

      return state.firstItemIndex;
    }, [items]);

    const components = React.useMemo(
      () => ({
        Footer: () => <div aria-hidden className={bottomFooterClassName} />,
        Header: topHeader
          ? () => <div className="flex flex-col gap-2 pb-2">{topHeader}</div>
          : undefined,
        List: TimelineList,
      }),
      [bottomFooterClassName, topHeader],
    );

    return (
      <Virtuoso<TimelineEntry>
        atBottomStateChange={atBottomStateChange}
        atBottomThreshold={32}
        className="h-full w-full"
        components={components}
        computeItemKey={(_, item) => item.key}
        data={items}
        data-scroll-restoration-id="virtualized-message-timeline"
        data-testid="message-timeline"
        defaultItemHeight={96}
        firstItemIndex={firstItemIndex}
        followOutput={followOutput}
        initialTopMostItemIndex={{ align: "end", index: items.length - 1 }}
        increaseViewportBy={{ bottom: 600, top: 900 }}
        itemContent={(_index, item) => {
          if (item.type === "day") {
            return <DayDivider label={item.label} />;
          }

          return renderTimelineMessageEntry({
            agentPubkeys,
            channelId,
            currentPubkey,
            entry: item,
            followThreadById,
            footer: messageFooters?.[item.message.id] ?? null,
            highlightedMessageId,
            isFollowingThreadById,
            onDelete,
            onEdit,
            onMarkUnread,
            onReply,
            onToggleReaction,
            personaLookup,
            profiles,
            searchActiveMessageId,
            searchMatchingMessageIds,
            searchQuery,
            unfollowThreadById,
            videoReviewContext: videoReviewContextById.get(item.message.id),
          });
        }}
        overscan={{ main: 800, reverse: 800 }}
        ref={virtuosoRef}
        scrollerRef={(element) => {
          scrollerRef?.(element instanceof HTMLDivElement ? element : null);
        }}
        startReached={() => {
          if (hasOlderMessages && !isFetchingOlder) {
            onStartReached?.();
          }
        }}
      />
    );
  },
);
