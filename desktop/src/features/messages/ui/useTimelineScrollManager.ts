import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { isNearBottom } from "./messageTimelineUtils";

type UseTimelineScrollManagerOptions = {
  channelId?: string | null;
  isLoading: boolean;
  messages: TimelineMessage[];
  onTargetReached?: (messageId: string) => void;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  targetMessageId?: string | null;
};

type PinToBottomOptions = {
  clearNewMessageCount?: boolean;
};

export function useTimelineScrollManager({
  channelId,
  isLoading,
  messages,
  onTargetReached,
  scrollContainerRef,
  targetMessageId,
}: UseTimelineScrollManagerOptions) {
  const timelineRef = scrollContainerRef;
  const contentRef = React.useRef<HTMLDivElement>(null);
  const bottomAnchorRef = React.useRef<HTMLDivElement>(null);
  const hasInitializedRef = React.useRef(false);
  const shouldStickToBottomRef = React.useRef(true);
  const isAtBottomRef = React.useRef(true);
  const isProgrammaticBottomScrollRef = React.useRef(false);
  const previousTimelineHeightRef = React.useRef<number | null>(null);
  const previousScrollTopRef = React.useRef(0);
  const lockedScrollTopRef = React.useRef<number | null>(null);
  const previousLastMessageIdRef = React.useRef<string | undefined>(undefined);
  const previousMessageCountRef = React.useRef(0);
  const handledTargetMessageIdRef = React.useRef<string | null>(null);
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);
  const [newMessageCount, setNewMessageCount] = React.useState(0);

  const resetScrollTracking = React.useCallback(() => {
    hasInitializedRef.current = false;
    shouldStickToBottomRef.current = true;
    isAtBottomRef.current = true;
    isProgrammaticBottomScrollRef.current = false;
    previousTimelineHeightRef.current = null;
    previousScrollTopRef.current = 0;
    lockedScrollTopRef.current = null;
    previousLastMessageIdRef.current = undefined;
    previousMessageCountRef.current = 0;
    handledTargetMessageIdRef.current = null;
    setIsAtBottom(true);
    setHighlightedMessageId(null);
    setNewMessageCount(0);
  }, []);

  const pinToBottom = React.useCallback(
    ({ clearNewMessageCount = false }: PinToBottomOptions = {}) => {
      shouldStickToBottomRef.current = true;
      isAtBottomRef.current = true;
      setIsAtBottom((current) => (current ? current : true));

      if (clearNewMessageCount) {
        setNewMessageCount(0);
      }
    },
    [],
  );

  const setObservedBottomState = React.useCallback((atBottom: boolean) => {
    shouldStickToBottomRef.current = atBottom;
    isAtBottomRef.current = atBottom;
    setIsAtBottom((current) => (current === atBottom ? current : atBottom));

    if (atBottom) {
      setNewMessageCount(0);
    }
  }, []);

  const unpinFromBottom = React.useCallback((scrollTop: number) => {
    shouldStickToBottomRef.current = false;
    isAtBottomRef.current = false;
    isProgrammaticBottomScrollRef.current = false;
    previousScrollTopRef.current = scrollTop;
    setIsAtBottom(false);
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is intentionally the sole trigger — we reset all scroll state when the channel changes
  React.useLayoutEffect(() => {
    resetScrollTracking();
  }, [channelId, resetScrollTracking]);

  const latestMessage =
    messages.length > 0 ? messages[messages.length - 1] : undefined;

  // biome-ignore lint/correctness/useExhaustiveDependencies: timelineRef is a stable React ref passed from the parent — its identity never changes
  const syncScrollState = React.useCallback(() => {
    const timeline = timelineRef.current;
    if (!timeline) {
      return;
    }

    const scrollTop = lockedScrollTopRef.current ?? timeline.scrollTop;
    const atBottom = isNearBottom(timeline);
    const movedAwayFromBottom = scrollTop + 1 < previousScrollTopRef.current;

    if (isProgrammaticBottomScrollRef.current) {
      previousScrollTopRef.current = scrollTop;

      if (movedAwayFromBottom) {
        isProgrammaticBottomScrollRef.current = false;
      } else if (!atBottom) {
        pinToBottom();
        return;
      } else {
        isProgrammaticBottomScrollRef.current = false;
        pinToBottom({ clearNewMessageCount: true });
        return;
      }
    }

    if (shouldStickToBottomRef.current && !atBottom && !movedAwayFromBottom) {
      previousScrollTopRef.current = scrollTop;
      pinToBottom({ clearNewMessageCount: true });
      return;
    }

    previousScrollTopRef.current = scrollTop;
    setObservedBottomState(atBottom);
  }, [pinToBottom, setObservedBottomState]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: timelineRef is a stable React ref — its identity never changes
  const restoreScrollPosition = React.useCallback(
    (scrollTop: number) => {
      const timeline = timelineRef.current;

      if (!timeline) {
        return;
      }

      isProgrammaticBottomScrollRef.current = false;
      lockedScrollTopRef.current = scrollTop;

      const restore = (remainingFrames: number) => {
        timeline.scrollTop = scrollTop;

        if (remainingFrames > 0) {
          requestAnimationFrame(() => {
            restore(remainingFrames - 1);
          });
          return;
        }

        lockedScrollTopRef.current = null;
        previousScrollTopRef.current = timeline.scrollTop;
        syncScrollState();
      };

      restore(2);
    },
    [syncScrollState],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: timelineRef is a stable React ref — its identity never changes
  const scrollToBottom = React.useCallback(
    (behavior: ScrollBehavior) => {
      const timeline = timelineRef.current;

      if (!timeline) {
        return;
      }

      isProgrammaticBottomScrollRef.current = true;

      const alignToBottom = (nextBehavior: ScrollBehavior) => {
        bottomAnchorRef.current?.scrollIntoView({
          block: "end",
          behavior: nextBehavior,
        });
        timeline.scrollTo({
          top: timeline.scrollHeight,
          behavior: nextBehavior,
        });
      };

      alignToBottom(behavior);
      lockedScrollTopRef.current = null;
      previousScrollTopRef.current = timeline.scrollTop;
      pinToBottom({ clearNewMessageCount: true });

      if (behavior === "smooth") {
        requestAnimationFrame(() => {
          previousScrollTopRef.current = timeline.scrollTop;
          syncScrollState();
        });
        return;
      }

      const settleAlignment = (remainingFrames: number) => {
        requestAnimationFrame(() => {
          alignToBottom("auto");
          previousScrollTopRef.current = timeline.scrollTop;

          if (remainingFrames > 0) {
            settleAlignment(remainingFrames - 1);
            return;
          }

          syncScrollState();
        });
      };

      settleAlignment(2);
    },
    [pinToBottom, syncScrollState],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: timelineRef is a stable React ref — its identity never changes
  React.useEffect(() => {
    const timeline = timelineRef.current;

    if (!timeline || typeof ResizeObserver === "undefined") {
      return;
    }

    previousTimelineHeightRef.current = timeline.clientHeight;
    previousScrollTopRef.current = timeline.scrollTop;

    const observer = new ResizeObserver(([entry]) => {
      const previousTimelineHeight = previousTimelineHeightRef.current;
      const nextTimelineHeight = entry.contentRect.height;
      previousTimelineHeightRef.current = nextTimelineHeight;

      if (
        previousTimelineHeight === null ||
        Math.abs(nextTimelineHeight - previousTimelineHeight) < 1
      ) {
        return;
      }

      if (shouldStickToBottomRef.current || isAtBottomRef.current) {
        scrollToBottom("auto");
        return;
      }

      restoreScrollPosition(previousScrollTopRef.current);
    });

    observer.observe(timeline);

    return () => {
      observer.disconnect();
    };
  }, [restoreScrollPosition, scrollToBottom]);

  React.useEffect(() => {
    const content = contentRef.current;

    if (!content || typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(() => {
      if (shouldStickToBottomRef.current) {
        scrollToBottom("auto");
        return;
      }

      syncScrollState();
    });

    observer.observe(content);

    return () => {
      observer.disconnect();
    };
  }, [scrollToBottom, syncScrollState]);

  React.useLayoutEffect(() => {
    if (!hasInitializedRef.current) {
      if (isLoading) {
        return;
      }

      if (targetMessageId) {
        const timeline = timelineRef.current;
        unpinFromBottom(timeline?.scrollTop ?? 0);
      } else {
        scrollToBottom("auto");
      }
      hasInitializedRef.current = true;
      previousLastMessageIdRef.current = latestMessage?.id;
      previousMessageCountRef.current = messages.length;
      return;
    }

    const previousLastMessageId = previousLastMessageIdRef.current;
    const previousMessageCount = previousMessageCountRef.current;
    const hasNewLatestMessage =
      latestMessage !== undefined && latestMessage.id !== previousLastMessageId;

    if (!hasNewLatestMessage) {
      previousLastMessageIdRef.current = latestMessage?.id;
      previousMessageCountRef.current = messages.length;
      return;
    }

    if (
      !targetMessageId &&
      (shouldStickToBottomRef.current ||
        isAtBottomRef.current ||
        latestMessage.accent)
    ) {
      scrollToBottom(latestMessage.accent ? "smooth" : "auto");
    } else {
      setNewMessageCount((current) => {
        const addedMessages = Math.max(
          1,
          messages.length - previousMessageCount,
        );
        return current + addedMessages;
      });
    }

    previousLastMessageIdRef.current = latestMessage.id;
    previousMessageCountRef.current = messages.length;
  }, [
    isLoading,
    latestMessage,
    messages.length,
    scrollToBottom,
    targetMessageId,
    timelineRef,
    unpinFromBottom,
  ]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: timelineRef is a stable React ref — its identity never changes
  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetMessageIdRef.current = null;
      setHighlightedMessageId(null);
      return;
    }

    if (handledTargetMessageIdRef.current === targetMessageId || isLoading) {
      return;
    }

    const timeline = timelineRef.current;
    if (!timeline) {
      return;
    }

    const targetElement = timeline.querySelector<HTMLElement>(
      `[data-message-id="${targetMessageId}"]`,
    );
    if (!targetElement) {
      return;
    }

    handledTargetMessageIdRef.current = targetMessageId;
    unpinFromBottom(timeline.scrollTop);
    setHighlightedMessageId(targetMessageId);
    setNewMessageCount(0);

    const alignToTarget = (remainingFrames: number) => {
      targetElement.scrollIntoView({
        block: "center",
        behavior: "auto",
      });
      previousScrollTopRef.current = timeline.scrollTop;

      if (remainingFrames > 0) {
        requestAnimationFrame(() => {
          alignToTarget(remainingFrames - 1);
        });
        return;
      }

      onTargetReached?.(targetMessageId);
    };

    alignToTarget(2);

    const timeout = window.setTimeout(() => {
      setHighlightedMessageId((current) =>
        current === targetMessageId ? null : current,
      );
    }, 2_000);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [isLoading, messages, onTargetReached, targetMessageId, unpinFromBottom]);

  return {
    bottomAnchorRef,
    contentRef,
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    restoreScrollPosition,
    scrollToBottom,
    syncScrollState,
  };
}
