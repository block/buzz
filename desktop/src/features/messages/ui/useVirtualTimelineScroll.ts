import * as React from "react";
import type { Virtualizer } from "@tanstack/react-virtual";

import {
  findVirtualRowIndexForMessage,
  type VirtualTimelineRow,
} from "@/features/messages/lib/buildVirtualTimelineRows";
import {
  isNearBottom,
  resolveDeepLinkTarget,
  selectLatestMessageKey,
} from "@/features/messages/lib/timelineSnapshot";
import type { TimelineMessage } from "@/features/messages/types";

type UseVirtualTimelineScrollOptions = {
  channelId?: string | null;
  isLoading: boolean;
  /** The deferred message snapshot the virtual rows are built from. */
  messages: TimelineMessage[];
  /** Flat virtual rows (dividers + messages) the virtualizer renders. */
  rows: VirtualTimelineRow[];
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  virtualizer: Virtualizer<HTMLDivElement, Element>;
  /**
   * True once the list's `scrollMargin` has been measured against a mounted
   * list. The first-load bottom pin waits on this: pinning while the margin is
   * still the pre-mount stale `0` lands `scrollMargin` px short of true bottom
   * and paints the rows out of place for a beat before re-anchoring.
   */
  scrollMarginReady: boolean;
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
  /** The currently active find-in-channel match, drives scroll-to-row. */
  searchActiveMessageId?: string | null;
};

/**
 * Scroll behavior for the virtualized main timeline. This REPLACES the bespoke
 * `useTimelineScrollManager`: the virtualizer owns the scroll container and all
 * measurement/anchoring, so the 400-line scrollTop-locking machinery
 * (`lockedScrollTopRef`, double-rAF restore, ResizeObserver re-pinning) is gone.
 *
 * What this hook keeps as a THIN wrapper layered on the virtualizer:
 *   - sticky-bottom autoscroll (`scrollToIndex(last, end)` when pinned)
 *   - `accent` smooth-scroll for highlighted messages
 *   - the `newMessageCount` "N new messages" pill when scrolled up
 *   - `isAtBottom` for the jump-to-latest affordance
 *   - deep-link + find-in-page jumps via `findVirtualRowIndexForMessage`
 *
 * Native key-stable retention (see `buildVirtualTimelineRows`) handles
 * scroll-up prepend, so there is no position-restore plumbing here.
 */
export function useVirtualTimelineScroll({
  channelId,
  isLoading,
  messages,
  rows,
  scrollContainerRef,
  virtualizer,
  scrollMarginReady,
  targetMessageId,
  onTargetReached,
  searchActiveMessageId,
}: UseVirtualTimelineScrollOptions) {
  const stickToBottomRef = React.useRef(true);
  const hasInitializedRef = React.useRef(false);
  const previousLastMessageKeyRef = React.useRef<string | undefined>(undefined);
  const previousMessageCountRef = React.useRef(0);
  const handledTargetMessageIdRef = React.useRef<string | null>(null);
  const handledSearchActiveIdRef = React.useRef<string | null>(null);
  // Total virtual size at the last time we pinned to bottom — lets the
  // settle-pin effect below re-anchor only when the size actually changed,
  // instead of looping on its own scroll-induced re-renders.
  const lastPinnedTotalSizeRef = React.useRef(-1);

  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [newMessageCount, setNewMessageCount] = React.useState(0);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);

  const lastRowIndex = rows.length - 1;

  const scrollToBottom = React.useCallback(
    (behavior: ScrollBehavior) => {
      if (lastRowIndex < 0) {
        return;
      }
      stickToBottomRef.current = true;
      setNewMessageCount(0);
      setIsAtBottom(true);
      virtualizer.scrollToIndex(lastRowIndex, { align: "end", behavior });
      // Mark the current size as pinned so the settle effect doesn't redundantly
      // re-fire for this same commit.
      lastPinnedTotalSizeRef.current = virtualizer.getTotalSize();
    },
    [lastRowIndex, virtualizer],
  );

  // Reset all scroll state when the channel changes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is intentionally the sole trigger
  React.useLayoutEffect(() => {
    hasInitializedRef.current = false;
    stickToBottomRef.current = true;
    previousLastMessageKeyRef.current = undefined;
    previousMessageCountRef.current = 0;
    handledTargetMessageIdRef.current = null;
    handledSearchActiveIdRef.current = null;
    lastPinnedTotalSizeRef.current = -1;
    setIsAtBottom(true);
    setNewMessageCount(0);
    setHighlightedMessageId(null);
  }, [channelId]);

  // Track bottom-pinned state off the native scroll event. The virtualizer owns
  // the scrollTop; we only read it to decide whether to keep auto-following.
  const syncScrollState = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) {
      return;
    }
    const atBottom = isNearBottom(container);
    stickToBottomRef.current = atBottom;
    setIsAtBottom((current) => (current === atBottom ? current : atBottom));
    if (atBottom) {
      setNewMessageCount(0);
    }
  }, [scrollContainerRef]);

  const latestMessage =
    messages.length > 0 ? messages[messages.length - 1] : undefined;
  const latestMessageKey = selectLatestMessageKey(messages);

  // Initial pin + new-message autoscroll. On first commit, jump to bottom (or
  // stay put for a deep-link). Afterwards, when a NEW latest message arrives:
  // autoscroll if pinned or accented, otherwise bump the "N new messages" pill.
  React.useLayoutEffect(() => {
    if (!hasInitializedRef.current) {
      // Wait for the first paint to settle: `isLoading` clearing means the rows
      // are mounting, but the list's `scrollMargin` is measured in a SIBLING
      // layout effect that races this one in the same commit. Pinning before
      // the margin lands anchors against the stale pre-mount `0`, landing
      // `scrollMargin` px short of true bottom — the out-of-place first-load
      // flash. Hold the init pin (without marking initialized) until the margin
      // is measured, so the very first pin lands against a trustworthy offset.
      if (isLoading || !scrollMarginReady) {
        return;
      }
      if (!targetMessageId) {
        scrollToBottom("auto");
      }
      hasInitializedRef.current = true;
      previousLastMessageKeyRef.current = latestMessageKey;
      previousMessageCountRef.current = messages.length;
      return;
    }

    const previousLastMessageKey = previousLastMessageKeyRef.current;
    const previousMessageCount = previousMessageCountRef.current;
    const hasNewLatestMessage =
      latestMessage !== undefined &&
      latestMessageKey !== previousLastMessageKey;

    if (!hasNewLatestMessage) {
      previousLastMessageKeyRef.current = latestMessageKey;
      previousMessageCountRef.current = messages.length;
      return;
    }

    if (
      !targetMessageId &&
      (stickToBottomRef.current || latestMessage.accent)
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

    previousLastMessageKeyRef.current = latestMessageKey;
    previousMessageCountRef.current = messages.length;
  }, [
    isLoading,
    latestMessage,
    latestMessageKey,
    messages.length,
    scrollMarginReady,
    scrollToBottom,
    targetMessageId,
  ]);

  // Keep pinned to the bottom while the document settles. On first load the
  // virtualizer paints with ESTIMATED row heights and the deferred snapshot
  // streams in over several commits, so the true bottom keeps moving after the
  // one-shot init pin above. As `getTotalSize()` grows (estimate→measured
  // heights, more rows, container resize), re-anchor to the bottom — but ONLY
  // while still pinned and not chasing a deep-link, so a user who scrolled up is
  // never yanked back down. Guarded on a real size change so the pin's own
  // scroll-induced re-render can't loop. This is what makes first-load "land and
  // hold at the bottom" instead of anchoring up top as content fills in.
  const totalSize = virtualizer.getTotalSize();
  React.useLayoutEffect(() => {
    if (
      isLoading ||
      targetMessageId ||
      !hasInitializedRef.current ||
      !stickToBottomRef.current ||
      lastRowIndex < 0
    ) {
      return;
    }
    if (totalSize === lastPinnedTotalSizeRef.current) {
      return;
    }
    lastPinnedTotalSizeRef.current = totalSize;
    virtualizer.scrollToIndex(lastRowIndex, { align: "end" });
  }, [totalSize, isLoading, targetMessageId, lastRowIndex, virtualizer]);

  // Deep-link jump-to-message. Drives the virtualizer to mount and center the
  // target row, replacing the bespoke querySelector + scrollIntoView path that
  // breaks under virtualization (the row may be unmounted).
  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetMessageIdRef.current = null;
      setHighlightedMessageId(null);
      return;
    }
    if (handledTargetMessageIdRef.current === targetMessageId || isLoading) {
      return;
    }
    // Only attempt once the target exists in THIS snapshot — same "bail and wait
    // for the next snapshot" contract the classic manager honored, which closes
    // the tearing race.
    if (!resolveDeepLinkTarget(messages, targetMessageId).resolved) {
      return;
    }
    const rowIndex = findVirtualRowIndexForMessage(
      rows,
      targetMessageId,
      messages,
    );
    if (rowIndex === -1) {
      return;
    }

    handledTargetMessageIdRef.current = targetMessageId;
    stickToBottomRef.current = false;
    setIsAtBottom(false);
    setHighlightedMessageId(targetMessageId);
    setNewMessageCount(0);
    virtualizer.scrollToIndex(rowIndex, { align: "center" });
    onTargetReached?.(targetMessageId);

    const timeout = window.setTimeout(() => {
      setHighlightedMessageId((current) =>
        current === targetMessageId ? null : current,
      );
    }, 2_000);
    return () => {
      window.clearTimeout(timeout);
    };
  }, [
    isLoading,
    messages,
    onTargetReached,
    rows,
    targetMessageId,
    virtualizer,
  ]);

  // Find-in-page: scroll the active search match into existence. Same
  // scrollToIndex bridge as deep-link; the row may be unmounted under
  // virtualization, so native browser find can't reach it.
  React.useEffect(() => {
    if (
      !searchActiveMessageId ||
      searchActiveMessageId === handledSearchActiveIdRef.current
    ) {
      handledSearchActiveIdRef.current = searchActiveMessageId ?? null;
      return;
    }
    handledSearchActiveIdRef.current = searchActiveMessageId;

    const rowIndex = findVirtualRowIndexForMessage(
      rows,
      searchActiveMessageId,
      messages,
    );
    if (rowIndex === -1) {
      return;
    }
    stickToBottomRef.current = false;
    virtualizer.scrollToIndex(rowIndex, {
      align: "center",
      behavior: "smooth",
    });
  }, [messages, rows, searchActiveMessageId, virtualizer]);

  return {
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    scrollToBottom,
    syncScrollState,
  };
}
