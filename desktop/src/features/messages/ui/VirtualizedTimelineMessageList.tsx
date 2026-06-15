import * as React from "react";
import {
  Virtuoso,
  type FollowOutput,
  type VirtuosoHandle,
} from "react-virtuoso";

import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { DayDivider } from "./DayDivider";
import {
  buildReviewCommentsByRootId,
  buildVideoReviewContextById,
  renderTimelineMessageEntry,
  type TimelineMessageListProps,
} from "./TimelineMessageList";

export type VirtualizedTimelineEntry =
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

export type VirtualizedTimelineModel = {
  items: VirtualizedTimelineEntry[];
  keyToRowIndex: Map<string, number>;
  messageIdToRowIndex: Map<string, number>;
};

type VirtualizedTimelineMessageListProps = TimelineMessageListProps & {
  atBottomStateChange?: (atBottom: boolean) => void;
  bottomFooterHeight?: number;
  firstItemIndex: number;
  followOutput?: FollowOutput;
  hasOlderMessages: boolean;
  items: VirtualizedTimelineEntry[];
  isFetchingOlder: boolean;
  onStartReached?: () => void;
  scrollerRef?: (element: HTMLDivElement | null) => void;
  topHeader?: React.ReactNode;
  virtuosoRef?: React.RefObject<VirtuosoHandle | null>;
};

const FIRST_ITEM_INDEX_BASE = 1_000_000;

export function useVirtualizedFirstItemIndex(
  items: readonly VirtualizedTimelineEntry[],
) {
  const firstItemIndexStateRef = React.useRef({
    firstItemIndex: FIRST_ITEM_INDEX_BASE,
    previousFirstMessageId: null as string | null,
    previousFirstMessageRowIndex: -1,
    previousItems: [] as readonly VirtualizedTimelineEntry[],
  });

  return React.useMemo(() => {
    const state = firstItemIndexStateRef.current;
    const previousItems = state.previousItems;
    const previousFirstMessageId = state.previousFirstMessageId;
    const previousFirstMessageRowIndex = state.previousFirstMessageRowIndex;
    const firstMessageRowIndex = items.findIndex(
      (item) => item.type === "message",
    );
    const firstMessage =
      firstMessageRowIndex >= 0 ? items[firstMessageRowIndex] : null;
    const firstMessageId =
      firstMessage?.type === "message" ? firstMessage.message.id : null;

    if (items.length === 0) {
      state.firstItemIndex = FIRST_ITEM_INDEX_BASE;
      state.previousFirstMessageId = null;
      state.previousFirstMessageRowIndex = -1;
      state.previousItems = items;
      return state.firstItemIndex;
    }

    if (previousItems !== items) {
      if (previousFirstMessageId && previousFirstMessageRowIndex >= 0) {
        const currentFirstLoadedMessageRowIndex = items.findIndex(
          (item) =>
            item.type === "message" &&
            item.message.id === previousFirstMessageId,
        );

        if (currentFirstLoadedMessageRowIndex >= 0) {
          state.firstItemIndex -=
            currentFirstLoadedMessageRowIndex - previousFirstMessageRowIndex;
        } else {
          state.firstItemIndex = FIRST_ITEM_INDEX_BASE;
        }
      }

      state.previousFirstMessageId = firstMessageId;
      state.previousFirstMessageRowIndex = firstMessageRowIndex;
      state.previousItems = items;
    }

    return state.firstItemIndex;
  }, [items]);
}

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

function getLocalDayKey(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1_000);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function buildVirtualizedTimelineModel(
  messages: TimelineMessage[],
): VirtualizedTimelineModel {
  const entries = buildMainTimelineEntries(messages);
  const items: VirtualizedTimelineEntry[] = [];
  const keyToRowIndex = new Map<string, number>();
  const messageIdToRowIndex = new Map<string, number>();
  let previousDayKey: string | null = null;

  const pushItem = (item: VirtualizedTimelineEntry) => {
    keyToRowIndex.set(item.key, items.length);
    if (item.type === "message") {
      messageIdToRowIndex.set(item.message.id, items.length);
    }
    items.push(item);
  };

  for (const { message, summary } of entries) {
    const dayKey = getLocalDayKey(message.createdAt);

    if (dayKey !== previousDayKey) {
      pushItem({
        key: `day:${dayKey}`,
        label: formatDayHeading(message.createdAt),
        type: "day",
      });
      previousDayKey = dayKey;
    }

    pushItem({
      key: `msg:${message.renderKey ?? message.id}`,
      message,
      summary,
      type: "message",
    });
  }

  return { items, keyToRowIndex, messageIdToRowIndex };
}

export const VirtualizedTimelineMessageList = React.memo(
  function VirtualizedTimelineMessageList({
    agentPubkeys,
    atBottomStateChange,
    bottomFooterHeight = 16,
    channelId,
    channelName,
    channelType,
    currentPubkey,
    firstItemIndex,
    followOutput = false,
    followThreadById,
    hasOlderMessages,
    highlightedMessageId = null,
    isFetchingOlder,
    isFollowingThreadById,
    messageFooters,
    messages,
    items,
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

    const canLoadOlder =
      hasOlderMessages && !isFetchingOlder && Boolean(onStartReached);
    const maybeLoadOlder = React.useCallback(() => {
      if (canLoadOlder) {
        onStartReached?.();
      }
    }, [canLoadOlder, onStartReached]);
    const [scrollerElement, setScrollerElement] =
      React.useState<HTMLDivElement | null>(null);

    React.useEffect(() => {
      if (!scrollerElement) return;

      const handleScroll = () => {
        if (scrollerElement.scrollTop <= 240) {
          maybeLoadOlder();
        }
      };

      scrollerElement.addEventListener("scroll", handleScroll, {
        passive: true,
      });
      return () => {
        scrollerElement.removeEventListener("scroll", handleScroll);
      };
    }, [maybeLoadOlder, scrollerElement]);
    const components = React.useMemo(
      () => ({
        Footer: () => (
          <div aria-hidden style={{ height: bottomFooterHeight }} />
        ),
        Header: topHeader
          ? () => <div className="flex flex-col gap-2 pb-2">{topHeader}</div>
          : undefined,
        List: TimelineList,
      }),
      [bottomFooterHeight, topHeader],
    );
    const [initialTopMostItemIndex] = React.useState(() => ({
      align: "end" as const,
      index: Math.max(0, items.length - 1),
    }));

    return (
      <Virtuoso<VirtualizedTimelineEntry>
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
        initialTopMostItemIndex={initialTopMostItemIndex}
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
          const scroller = element instanceof HTMLDivElement ? element : null;
          setScrollerElement(scroller);
          scrollerRef?.(scroller);
        }}
        startReached={maybeLoadOlder}
      />
    );
  },
);
