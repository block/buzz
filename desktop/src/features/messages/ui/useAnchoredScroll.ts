import * as React from "react";

import { classifyTimelineMessageDelta } from "@/features/messages/lib/timelineSnapshot";
import {
  getPinnedCenterDrift,
  settleProgrammaticBottomPin,
  shouldIgnorePinnedCenterScroll,
  shouldSettleForSplitPanel,
  shouldSettleVirtualizedBottom,
} from "./anchoredScrollPolicy";
import {
  getTargetRowCenterOffset,
  isTargetRowCentered,
  targetRowNeedsCenterCorrection,
} from "./targetRowCentering";
import type {
  AnchorState,
  ScrollToMessageResult,
  UseAnchoredScrollOptions,
  UseAnchoredScrollResult,
} from "./anchoredScrollTypes";
import { useVirtualizedViewportResize } from "./useVirtualizedViewportResize";

/**
 * Distance (in CSS pixels) below which we consider the scroll position
 * "at the bottom" of the message list. Tight enough that the user has to
 * actually scroll down to re-pin; permissive enough to tolerate sub-pixel
 * rounding from the layout engine.
 */
const AT_BOTTOM_THRESHOLD_PX = 32;

function isAtBottomNow(
  container: Pick<
    HTMLDivElement,
    "scrollHeight" | "clientHeight" | "scrollTop"
  >,
) {
  return (
    container.scrollHeight - container.clientHeight - container.scrollTop <=
    AT_BOTTOM_THRESHOLD_PX
  );
}

/**
 * Pick an anchor for the current scroll position.
 *
 * Top-crossing walk: chronological children, top-down. The first
 * `data-message-id` row whose bottom edge has crossed below the container
 * top is the anchor — that's the row the reader's eye is on when they've
 * scrolled up through history. `topOffset` is the row's top relative to
 * the container's top and may be negative when the row straddles the edge.
 *
 * If no such row exists (e.g. nothing scrolled past the top, list shorter
 * than the viewport, etc.) the anchor is `at-bottom`.
 *
 * Algorithm credit: Sami's [13] in the buzz-bugs scroll-redesign thread,
 * supersedes the Matrix-style bottom-up walk in [7]. The top-crossing
 * choice is what keeps the row the reader is *reading* fixed under
 * in-viewport reflow (image-load, embed expansion).
 */
function computeAnchor(
  container: HTMLDivElement,
  treatNearBottomAsBottom = true,
): AnchorState {
  if (treatNearBottomAsBottom && isAtBottomNow(container)) {
    return { kind: "at-bottom" };
  }

  const containerTop = container.getBoundingClientRect().top;
  const rows = container.querySelectorAll<HTMLElement>("[data-message-id]");

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    const rect = row.getBoundingClientRect();
    if (rect.bottom > containerTop) {
      const messageId = row.dataset.messageId;
      if (messageId) {
        return {
          kind: "message",
          messageId,
          topOffset: rect.top - containerTop,
        };
      }
    }
  }

  return { kind: "at-bottom" };
}

export function useAnchoredScroll({
  scrollContainerRef,
  contentRef,
  channelId,
  isLoading,
  messages,
  splitPanelOpen = false,

  targetMessageId = null,
  highlightTargetMessage = true,
  pinTargetCentered = false,
  topBoundaryReached = false,
  onTargetReached,
  onTargetSettled,
  virtualCancelBottomIntent,
  virtualScrollBy,
  virtualScrollToMessage,
  virtualScrollToBottom,
  virtualSettleAtBottom,
  virtualizerOwnsPrependAnchoring = false,
  virtualizerRenderVersion = 0,
}: UseAnchoredScrollOptions): UseAnchoredScrollResult {
  // Anchor lives in a ref because it must survive renders and is updated
  // both on scroll (commit-time read) and in the layout effect (post-render
  // restoration). useState would force re-renders we don't want.
  const anchorRef = React.useRef<AnchorState>({ kind: "at-bottom" });
  const virtualizerAtBottomRef = React.useRef(true);
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  React.useLayoutEffect(() => {
    if (shouldSettleForSplitPanel({ isAtBottom, splitPanelOpen })) {
      virtualSettleAtBottom?.();
    }
  }, [isAtBottom, splitPanelOpen, virtualSettleAtBottom]);
  const [newMessageCount, setNewMessageCount] = React.useState(0);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);

  const hasInitializedRef = React.useRef(false);
  const prevLastMessageIdRef = React.useRef<string | undefined>(undefined);
  const prevFirstMessageIdRef = React.useRef<string | undefined>(undefined);
  const prevMessageCountRef = React.useRef(0);
  const prevMessagesRef = React.useRef<Array<{ id: string }>>([]);
  const handledTargetIdRef = React.useRef<string | null>(null);
  const highlightTimeoutRef = React.useRef<number | null>(null);
  // Tracks a pending rAF queued by pinToBottomOnMount so it can be cancelled
  // on channel switch (the channelId reset effect clears it).
  const mountPinRafIdRef = React.useRef<number | null>(null);
  // One-shot: the consumer calls `scrollToBottomOnNextUpdate()` right before
  // it sends a message (see ChannelPane). When the user's own message then
  // appends, we snap to bottom even if they had scrolled up to read history.
  // Consumed (and cleared) by the next append in the restoration effect.
  const forceBottomOnNextAppendRef = React.useRef(false);
  // True from a programmatic bottom pin until the list's row measurement settles
  // and the view reaches a true physical bottom. During this window `onScroll`
  // ignores transient gaps and keeps chasing the floor. A `ref`, not state — the
  // guard runs on a native scroll event, outside React's render cycle.
  const settlingRef = React.useRef(false);
  // Pinned-center corrections write scroll position themselves. Keep the next
  // matching scroll event from being mistaken for a user releasing the pin.
  const programmaticScrollTopRef = React.useRef<number | null>(null);
  const isWritingScrollRef = React.useRef(false);
  const programmaticScrollRafRef = React.useRef<number | null>(null);
  const targetSettleRafRef = React.useRef<number | null>(null);
  const targetRetryRafRef = React.useRef<number | null>(null);
  const virtualTargetJumpRef = React.useRef<{
    messageId: string;
    messageCount: number;
  } | null>(null);
  const virtualTargetCorrectionAppliedRef = React.useRef(false);
  const targetCorrectionRafRef = React.useRef<number | null>(null);
  const [targetRetryVersion, setTargetRetryVersion] = React.useState(0);

  // Reset everything when the channel changes — the layout effect that runs
  // immediately after this reset is responsible for either jumping to bottom
  // or to the target message for the new channel.
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is intentionally the sole trigger — we want this effect to fire exactly when the channel changes (and on mount).
  React.useLayoutEffect(() => {
    anchorRef.current = { kind: "at-bottom" };
    virtualizerAtBottomRef.current = true;
    setIsAtBottom(true);
    setNewMessageCount(0);
    setHighlightedMessageId(null);
    hasInitializedRef.current = false;
    prevLastMessageIdRef.current = undefined;
    prevFirstMessageIdRef.current = undefined;
    prevMessageCountRef.current = 0;
    prevMessagesRef.current = [];
    handledTargetIdRef.current = null;
    forceBottomOnNextAppendRef.current = false;
    settlingRef.current = false;
    programmaticScrollTopRef.current = null;
    isWritingScrollRef.current = false;
    if (programmaticScrollRafRef.current !== null) {
      cancelAnimationFrame(programmaticScrollRafRef.current);
      programmaticScrollRafRef.current = null;
    }
    if (targetSettleRafRef.current !== null) {
      cancelAnimationFrame(targetSettleRafRef.current);
      targetSettleRafRef.current = null;
    }
    if (targetRetryRafRef.current !== null) {
      cancelAnimationFrame(targetRetryRafRef.current);
      targetRetryRafRef.current = null;
    }
    if (targetCorrectionRafRef.current !== null) {
      cancelAnimationFrame(targetCorrectionRafRef.current);
      targetCorrectionRafRef.current = null;
    }
    virtualTargetJumpRef.current = null;
    virtualTargetCorrectionAppliedRef.current = false;
    if (highlightTimeoutRef.current !== null) {
      window.clearTimeout(highlightTimeoutRef.current);
      highlightTimeoutRef.current = null;
    }
    if (mountPinRafIdRef.current !== null) {
      cancelAnimationFrame(mountPinRafIdRef.current);
      mountPinRafIdRef.current = null;
    }
  }, [channelId]);

  const noteProgrammaticScroll = React.useCallback(
    (container: HTMLDivElement, scrollTopBefore: number) => {
      if (scrollTopBefore === container.scrollTop) return;

      programmaticScrollTopRef.current = container.scrollTop;
      if (programmaticScrollRafRef.current !== null) {
        cancelAnimationFrame(programmaticScrollRafRef.current);
      }
      // A programmatic scroll event is delivered before the next frame. If the
      // browser does not emit one, expire the guard so a later user scroll is
      // never swallowed.
      programmaticScrollRafRef.current = requestAnimationFrame(() => {
        if (programmaticScrollTopRef.current === container.scrollTop) {
          programmaticScrollTopRef.current = null;
        }
        programmaticScrollRafRef.current = null;
      });
    },
    [],
  );

  const writePinnedCenterScroll = React.useCallback(
    (container: HTMLDivElement, write: () => void) => {
      const scrollTopBefore = container.scrollTop;
      isWritingScrollRef.current = true;
      write();
      isWritingScrollRef.current = false;
      noteProgrammaticScroll(container, scrollTopBefore);
    },
    [noteProgrammaticScroll],
  );

  const repinPinnedCenter = React.useCallback(() => {
    const anchor = anchorRef.current;
    const container = scrollContainerRef.current;
    if (anchor.kind !== "pinned-center" || !container) return;

    const row = container.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
    );
    if (!row) return;

    const currentContentTop =
      row.getBoundingClientRect().top +
      container.scrollTop -
      container.getBoundingClientRect().top;
    const drift = getPinnedCenterDrift({
      contentTop: anchor.contentTop,
      currentContentTop,
    });
    if (drift === null) return;

    anchor.contentTop = currentContentTop;
    writePinnedCenterScroll(container, () => container.scrollBy(0, drift));
  }, [scrollContainerRef, writePinnedCenterScroll]);

  const releasePinnedCenter = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container || anchorRef.current.kind !== "pinned-center") return;

    // A selected row can sit near the physical floor after its deliberate
    // center. A direct user scroll there must still release the center pin;
    // otherwise a passive representative update is mistaken for bottom glue.
    anchorRef.current = computeAnchor(container, false);
    const atBottom = isAtBottomNow(container);
    setIsAtBottom((previous) => (previous === atBottom ? previous : atBottom));
    if (atBottom) setNewMessageCount(0);
  }, [scrollContainerRef]);

  const schedulePinnedTargetSettle = React.useCallback(
    (messageId: string) => {
      if (!onTargetSettled) return;
      if (targetSettleRafRef.current !== null) {
        cancelAnimationFrame(targetSettleRafRef.current);
      }
      targetSettleRafRef.current = requestAnimationFrame(() => {
        targetSettleRafRef.current = null;
        const container = scrollContainerRef.current;
        const anchor = anchorRef.current;
        if (
          !container ||
          anchor.kind !== "pinned-center" ||
          anchor.messageId !== messageId
        ) {
          return;
        }
        const row = container.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(messageId)}"]`,
        );
        if (!row) return;
        const rowRect = row.getBoundingClientRect();
        const containerRect = container.getBoundingClientRect();
        if (
          rowRect.bottom > containerRect.top &&
          rowRect.top < containerRect.bottom
        ) {
          onTargetSettled(messageId);
        }
      });
    },
    [onTargetSettled, scrollContainerRef],
  );

  const scrollToBottomImperative = React.useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const container = scrollContainerRef.current;
      if (!container) return;
      anchorRef.current = { kind: "at-bottom" };
      // A programmatic jump-to-bottom is not atomic, even for `behavior: "auto"`:
      // the browser can emit `scroll` while the list is still settling row
      // measurements. During that window `computeAnchor` may read the transient
      // gap as a deliberate scroll-up and latch a mid-history message anchor,
      // which strands future appends above the floor. Arm the settle guard for
      // every imperative bottom jump so `onScroll` holds the at-bottom anchor
      // until it can snap to the true floor.
      settlingRef.current = true;
      if (virtualizerOwnsPrependAnchoring && virtualScrollToBottom) {
        virtualScrollToBottom(behavior);
      } else {
        container.scrollTo({ top: container.scrollHeight, behavior });
      }
      setIsAtBottom(true);
      setNewMessageCount(0);
    },
    [
      scrollContainerRef,
      virtualScrollToBottom,
      virtualizerOwnsPrependAnchoring,
    ],
  );

  // Arm a one-shot: the next append snaps to bottom regardless of where the
  // user is. The consumer calls this right before sending so their own
  // outbound message pulls the view down even if they'd scrolled up.
  const scrollToBottomOnNextUpdate = React.useCallback(() => {
    forceBottomOnNextAppendRef.current = true;
  }, []);

  const settleAtBottomAfterLayout = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return false;
    if (anchorRef.current.kind === "pinned-center") {
      repinPinnedCenter();
      const atBottom = isAtBottomNow(container);
      setIsAtBottom((previous) =>
        previous === atBottom ? previous : atBottom,
      );
      if (atBottom) setNewMessageCount(0);
      schedulePinnedTargetSettle(anchorRef.current.messageId);
      return true;
    }
    if (!isAtBottomNow(container)) return false;

    anchorRef.current = { kind: "at-bottom" };
    setIsAtBottom(true);
    setNewMessageCount(0);
    if (!virtualizerOwnsPrependAnchoring) {
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
    }
    return true;
  }, [
    repinPinnedCenter,
    schedulePinnedTargetSettle,
    scrollContainerRef,
    virtualizerOwnsPrependAnchoring,
  ]);

  const highlightMessage = React.useCallback((messageId: string) => {
    if (highlightTimeoutRef.current !== null) {
      window.clearTimeout(highlightTimeoutRef.current);
    }
    setHighlightedMessageId(messageId);
    highlightTimeoutRef.current = window.setTimeout(() => {
      setHighlightedMessageId((current) =>
        current === messageId ? null : current,
      );
      highlightTimeoutRef.current = null;
    }, 2_000);
  }, []);

  const scrollToMessageImperative = React.useCallback(
    (
      messageId: string,
      options: { highlight?: boolean; behavior?: ScrollBehavior } = {},
    ): ScrollToMessageResult => {
      const container = scrollContainerRef.current;
      if (!container) return "missing";
      const el = container.querySelector<HTMLElement>(
        `[data-message-id="${messageId}"]`,
      );
      if (virtualizerOwnsPrependAnchoring && !virtualScrollToMessage)
        return "pending"; // Wait for Virtua's imperative API.
      if (virtualizerOwnsPrependAnchoring && virtualScrollToMessage) {
        // Virtua is the sole scroll writer; a new jump cancels its initial
        // bottom settle and the cancel call prevents later re-pinning.
        virtualCancelBottomIntent?.();
        const virtualScrollBehavior = el
          ? (options.behavior ?? "auto")
          : "auto";
        const rowIsVisible = el
          ? (() => {
              const rowRect = el.getBoundingClientRect();
              const containerRect = container.getBoundingClientRect();
              return (
                rowRect.bottom > containerRect.top &&
                rowRect.top < containerRect.bottom
              );
            })()
          : false;
        const jumpMatchesCurrentModel =
          virtualTargetJumpRef.current?.messageId === messageId &&
          virtualTargetJumpRef.current.messageCount === messages.length;
        if (!jumpMatchesCurrentModel) {
          if (
            !virtualScrollToMessage(messageId, {
              behavior: virtualScrollBehavior,
            })
          ) {
            return "missing";
          }
          virtualTargetJumpRef.current = {
            messageId,
            messageCount: messages.length,
          };
          virtualTargetCorrectionAppliedRef.current = false;
        }
        if (
          rowIsVisible &&
          !virtualTargetCorrectionAppliedRef.current &&
          targetCorrectionRafRef.current === null
        ) {
          // Virtua first realizes and centers by index. Once the target row is
          // rendered, wait one more frame for its measured geometry to land,
          // then compensate for chrome that overlays the usable viewport.
          targetCorrectionRafRef.current = requestAnimationFrame(() => {
            targetCorrectionRafRef.current = null;
            const settledContainer = scrollContainerRef.current;
            const settledRow = settledContainer?.querySelector<HTMLElement>(
              `[data-message-id="${CSS.escape(messageId)}"]`,
            );
            if (!settledContainer || !settledRow) return;
            const rowRect = settledRow.getBoundingClientRect();
            const containerRect = settledContainer.getBoundingClientRect();
            // Virtua may mount the requested row before its indexed jump has
            // placed that row in the viewport. Do not turn that transient,
            // offscreen geometry into a multi-thousand-pixel correction.
            if (
              rowRect.bottom <= containerRect.top ||
              rowRect.top >= containerRect.bottom
            ) {
              setTargetRetryVersion((version) => version + 1);
              return;
            }
            const correction = getTargetRowCenterOffset(
              settledRow,
              settledContainer,
            );
            if (targetRowNeedsCenterCorrection(correction)) {
              virtualScrollBy?.(correction);
            }
            virtualTargetCorrectionAppliedRef.current = true;
            setTargetRetryVersion((version) => version + 1);
          });
        }
        anchorRef.current = { kind: "message", messageId, topOffset: 0 };
        // Completion requires midpoint alignment or a confirmed boundary.
        const targetIndex = messages.findIndex(
          (message) => message.id === messageId,
        );
        const targetBoundary =
          targetIndex === 0 && topBoundaryReached
            ? "top"
            : targetIndex === messages.length - 1
              ? "bottom"
              : "none";
        if (
          !el ||
          !isTargetRowCentered(el, container, targetBoundary, isAtBottomNow)
        ) {
          virtualizerAtBottomRef.current = false;
          setIsAtBottom(false);
          return "pending";
        }
        const atBottom = isAtBottomNow(container);
        virtualizerAtBottomRef.current = atBottom;
        setIsAtBottom(atBottom);
        if (options.highlight) highlightMessage(messageId);
        virtualTargetJumpRef.current = null;
        virtualTargetCorrectionAppliedRef.current = false;
        return "centered";
      }

      if (!el) return "missing";

      const rect = el.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const currentTopOffset = rect.top - containerRect.top;
      const centeredTopOffset = (container.clientHeight - rect.height) / 2;
      const maxScrollTop = Math.max(
        0,
        container.scrollHeight - container.clientHeight,
      );
      const targetScrollTop = Math.min(
        maxScrollTop,
        Math.max(0, container.scrollTop + currentTopOffset - centeredTopOffset),
      );
      const targetTopOffset =
        currentTopOffset - (targetScrollTop - container.scrollTop);
      const contentTop = rect.top + container.scrollTop - containerRect.top;

      if (pinTargetCentered) {
        writePinnedCenterScroll(container, () => {
          el.scrollIntoView({
            block: "center",
            behavior: options.behavior ?? "auto",
          });
        });
        anchorRef.current = {
          kind: "pinned-center",
          messageId,
          contentTop,
        };
        setIsAtBottom(isAtBottomNow(container));
      } else {
        container.scrollTo({
          top: targetScrollTop,
          behavior: options.behavior ?? "auto",
        });

        // Smooth scrolling starts an async animation, so measuring after the call can still return the pre-animation position.
        // Save the clamped destination offset instead; otherwise a concurrent
        // render/ResizeObserver restore can fight the smooth scroll back toward
        // where it started.
        anchorRef.current = {
          kind: "message",
          messageId,
          topOffset: targetTopOffset,
        };
      }
      if (!pinTargetCentered) {
        setIsAtBottom(maxScrollTop - targetScrollTop <= AT_BOTTOM_THRESHOLD_PX);
      }

      if (options.highlight) highlightMessage(messageId);
      return "centered";
    },
    [
      highlightMessage,
      messages,
      pinTargetCentered,
      topBoundaryReached,
      scrollContainerRef,
      virtualCancelBottomIntent,
      virtualScrollBy,
      virtualizerOwnsPrependAnchoring,
      writePinnedCenterScroll,
      virtualScrollToMessage,
    ],
  );

  // Scroll handler: recompute anchor + bottom state from the current
  // scroll position. Cheap enough to run on every scroll event — a single
  // `getBoundingClientRect` walk plus rect reads.
  const onScroll = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    // Virtua owns anchoring and reports bottom state separately. Avoid the
    // fallback's O(N) DOM walk on every compositor-driven scroll event.
    if (virtualizerOwnsPrependAnchoring) return;
    // Row measurement can grow `scrollHeight` after a bottom pin and emit scroll
    // events while `scrollTop` holds at the old floor — opening a transient gap
    // above the true bottom. `computeAnchor` would read that as a deliberate
    // scroll-up and latch a message anchor, freezing the view short of bottom.
    // While settling, keep the anchor at-bottom and chase the physical floor.
    if (settlingRef.current) {
      if (settleProgrammaticBottomPin(container)) {
        settlingRef.current = false;
      } else {
        if (virtualizerOwnsPrependAnchoring) {
          settlingRef.current = false;
        }
        return;
      }
    }
    if (anchorRef.current.kind === "pinned-center") {
      if (
        shouldIgnorePinnedCenterScroll({
          currentScrollTop: container.scrollTop,
          expectedScrollTop: programmaticScrollTopRef.current,
          isWritingScroll: isWritingScrollRef.current,
        })
      ) {
        if (programmaticScrollTopRef.current === container.scrollTop) {
          programmaticScrollTopRef.current = null;
        }
        return;
      }
      releasePinnedCenter();
      return;
    }
    anchorRef.current = computeAnchor(container);
    const atBottom = anchorRef.current.kind === "at-bottom";
    setIsAtBottom((prev) => (prev === atBottom ? prev : atBottom));
    if (atBottom) {
      setNewMessageCount(0);
    }
  }, [
    releasePinnedCenter,
    scrollContainerRef,
    virtualizerOwnsPrependAnchoring,
  ]);

  // ---------------------------------------------------------------------------
  // Anchor restoration: after every render, stick to the bottom if the user is
  // there. The reading position across prepend / in-viewport reflow is held by
  // the browser's native scroll anchoring (overflow-anchor) now that every
  // loaded row stays in the DOM, so there is no JS message-anchor restore.
  // ---------------------------------------------------------------------------

  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    // First render after a reset (channel switch or initial mount): jump
    // to the requested target message, or to the bottom by default.
    if (!hasInitializedRef.current) {
      if (isLoading) return;
      // The virtualized list owns the actual scroll node. Its API registers in
      // a child layout effect, after this parent hook's first pass; treating
      // that API-less pass as initialized writes to the inert outer wrapper
      // and permanently consumes the channel's initial bottom pin.
      if (virtualizerOwnsPrependAnchoring && !virtualScrollToBottom) return;
      // Establish the initial position before the browser paints. The follow-up
      // frame is a settling pass for content whose measurements land with the
      // commit (fonts, deferred rows, media), not the first bottom pin. Keeping
      // both writes in the shared scroll owner gives every conversation surface
      // the same first-frame behavior regardless of its surrounding animation.
      const pinToBottomOnMount = () => {
        scrollToBottomImperative("auto");
        mountPinRafIdRef.current = requestAnimationFrame(() => {
          mountPinRafIdRef.current = null;
          scrollToBottomImperative("auto");
        });
      };
      if (targetMessageId) {
        // A cold deep-link target is rarely in the DOM on this first commit:
        // a virtualized list renders only a window, and a target outside the
        // loaded history is fetched by id and spliced in a render or two
        // later. Either way the post-mount target effect (keyed on `messages`
        // and the rendered range) finishes the job — it is not handled yet.
        // Only fall back to the bottom pin when the row is genuinely absent;
        // pinning while the virtualizer is mid-jump re-arms durable bottom
        // intent and strands the view at the floor with no highlight.
        const result = scrollToMessageImperative(targetMessageId, {
          highlight: highlightTargetMessage,
        });
        if (result === "centered") {
          handledTargetIdRef.current = targetMessageId;
          onTargetReached?.(targetMessageId);
        } else if (result === "missing") {
          pinToBottomOnMount();
        }
      } else {
        pinToBottomOnMount();
      }
      hasInitializedRef.current = true;
      prevLastMessageIdRef.current = messages[messages.length - 1]?.id;
      prevFirstMessageIdRef.current = messages[0]?.id;
      prevMessageCountRef.current = messages.length;
      prevMessagesRef.current = messages;
      return;
    }

    const anchor = anchorRef.current;
    const lastMessage = messages[messages.length - 1];
    const firstMessage = messages[0];
    const prevLastId = prevLastMessageIdRef.current;
    const prevCount = prevMessageCountRef.current;
    const newLatestArrived =
      lastMessage !== undefined && lastMessage.id !== prevLastId;
    // Count growth, not tail-id change, is the reliable "messages arrived"
    // signal. The relay can deliver a message that sorts ahead of an existing
    // same-second row, so the list grows without the *last* id changing —
    // `newLatestArrived` misses that case and the unread counter never bumps.
    const prevMessages = prevMessagesRef.current;
    const messagesArrived = messages.length - prevCount;
    const messageDelta = classifyTimelineMessageDelta({
      current: messages,
      previous: prevMessages,
    });
    const isPrepend = messageDelta === "prepend";

    // One-shot: an outbound send armed `scrollToBottomOnNextUpdate`. When the
    // resulting append lands, snap to bottom regardless of the current anchor,
    // then clear the flag. Bail before the anchored branch so the user's own
    // message pulls the view down.
    if (newLatestArrived && forceBottomOnNextAppendRef.current) {
      forceBottomOnNextAppendRef.current = false;
      anchorRef.current = { kind: "at-bottom" };
      settlingRef.current = true;
      if (virtualizerOwnsPrependAnchoring && virtualScrollToBottom) {
        virtualScrollToBottom("auto");
      } else {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      }
      setIsAtBottom(true);
      setNewMessageCount(0);
      prevLastMessageIdRef.current = lastMessage?.id;
      prevFirstMessageIdRef.current = firstMessage?.id;
      prevMessageCountRef.current = messages.length;
      prevMessagesRef.current = messages;
      return;
    }

    if (anchor.kind === "pinned-center") {
      repinPinnedCenter();
    } else if (anchor.kind === "at-bottom") {
      if (
        virtualizerOwnsPrependAnchoring &&
        shouldSettleVirtualizedBottom({
          isAtBottom: virtualizerAtBottomRef.current,
          messageDelta,
          messagesArrived,
          messagesChanged: messages !== prevMessages,
        })
      ) {
        virtualSettleAtBottom?.();
      } else if (!virtualizerOwnsPrependAnchoring) {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      }
      if (newLatestArrived) setNewMessageCount(0);
    } else if (
      messagesArrived > 0 &&
      !targetMessageId &&
      !virtualizerOwnsPrependAnchoring &&
      isAtBottomNow(container)
    ) {
      // A native scroll/layout callback may not have reconciled a stale
      // message anchor before this append commits. If the rendered result is
      // still physically at the floor (common in short threads), do not turn
      // that stale anchor into a visible unread affordance. Active navigation
      // targets own the viewport and must be preserved across presentation
      // reflow even when the old geometry momentarily reads as the floor.
      anchorRef.current = { kind: "at-bottom" };
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      setIsAtBottom(true);
      setNewMessageCount(0);
    } else if (messagesArrived > 0 && !virtualizerOwnsPrependAnchoring) {
      // Anchored mid-history. An older-history prepend grows the content above
      // the reading row; the browser's native scroll anchoring does NOT correct
      // this at the top edge (no anchor node above the viewport when scrollTop
      // is ~0), so re-pin the anchored row to its saved offset by id. This is
      // the single scroll writer for the prepend — the load-older observer only
      // triggers the fetch. We run it in this post-commit layout effect (not the
      // observer's promise callback) because the prepended rows commit on a
      // deferred snapshot a few frames later, so the row's true position is only
      // known here.
      const row = container.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
      );
      if (row) {
        const currentTopOffset =
          row.getBoundingClientRect().top -
          container.getBoundingClientRect().top;
        const drift = currentTopOffset - anchor.topOffset;
        if (Math.abs(drift) > 0.5) {
          container.scrollBy(0, drift);
        }
      }
      if (!isPrepend) {
        setNewMessageCount((current) => current + messagesArrived);
      }
    }

    prevLastMessageIdRef.current = lastMessage?.id;
    prevFirstMessageIdRef.current = firstMessage?.id;
    prevMessageCountRef.current = messages.length;
    prevMessagesRef.current = messages;
  }, [
    highlightTargetMessage,
    isLoading,
    messages,
    onTargetReached,
    repinPinnedCenter,
    scrollContainerRef,
    scrollToBottomImperative,
    scrollToMessageImperative,
    targetMessageId,
    virtualScrollToBottom,
    virtualSettleAtBottom,
    virtualizerOwnsPrependAnchoring,
  ]);

  // ---------------------------------------------------------------------------
  // Content resize: while stuck to the bottom, an in-viewport reflow (image
  // decode, embed expand, late font load) that React isn't driving grows
  // `scrollHeight` without a `messages` change, so the layout effect doesn't
  // fire — re-pin to the new floor here to stay glued. When anchored
  // mid-history, native scroll anchoring (overflow-anchor) holds the reading
  // row across the reflow, so there's nothing to do.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId deliberately re-subscribes after a keyed or conditional scroll-content mount replaces ref.current.
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const container = scrollContainerRef.current;
      if (!container) return;
      if (settleAtBottomAfterLayout()) return;
      if (
        anchorRef.current.kind === "at-bottom" &&
        !virtualizerOwnsPrependAnchoring
      ) {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      }
    });
    observer.observe(content);
    const container = scrollContainerRef.current;
    if (container && container !== content) observer.observe(container);
    return () => {
      observer.disconnect();
      if (targetSettleRafRef.current !== null) {
        cancelAnimationFrame(targetSettleRafRef.current);
        targetSettleRafRef.current = null;
      }
    };
  }, [
    channelId,
    contentRef,
    scrollContainerRef,
    settleAtBottomAfterLayout,
    virtualizerOwnsPrependAnchoring,
  ]);

  useVirtualizedViewportResize(
    scrollContainerRef,
    virtualizerAtBottomRef,
    virtualizerOwnsPrependAnchoring ? virtualSettleAtBottom : undefined,
  );

  // Pinned centers survive our own corrections but release as soon as the
  // reader deliberately takes control of the scroll position or the caller
  // retires the temporary target after layout settlement.
  React.useEffect(() => {
    if (!pinTargetCentered) releasePinnedCenter();
  }, [pinTargetCentered, releasePinnedCenter]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId deliberately re-subscribes after a keyed or conditional scroll-container mount replaces ref.current.
  React.useEffect(() => {
    if (!pinTargetCentered) return;
    const container = scrollContainerRef.current;
    if (!container) return;

    const handleUserInteraction = () => {
      const pinnedMessageId =
        anchorRef.current.kind === "pinned-center"
          ? anchorRef.current.messageId
          : null;
      releasePinnedCenter();
      if (pinnedMessageId) onTargetSettled?.(pinnedMessageId);
    };
    container.addEventListener("wheel", handleUserInteraction, {
      passive: true,
    });
    container.addEventListener("touchstart", handleUserInteraction, {
      passive: true,
    });
    container.addEventListener("keydown", handleUserInteraction);
    return () => {
      container.removeEventListener("wheel", handleUserInteraction);
      container.removeEventListener("touchstart", handleUserInteraction);
      container.removeEventListener("keydown", handleUserInteraction);
    };
  }, [
    channelId,
    onTargetSettled,
    pinTargetCentered,
    releasePinnedCenter,
    scrollContainerRef,
  ]);

  // ---------------------------------------------------------------------------
  // Target message handling (deep link, jump-to-reply, etc.). Distinct from
  // the initial-mount target above — this handles changes after the first
  // render.
  //
  // A deep-link target may live in older history that isn't in the DOM when
  // the route param first changes. The route screen fetches the target event
  // by id and splices it into `messages` asynchronously, so its row appears a
  // render or two later. We therefore key this effect on `messages` and bail
  // *without* marking the target handled until its row actually exists — each
  // subsequent message commit re-runs the effect and retries the centering.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: `messages` and `virtualizerRenderVersion` are intentional retry triggers, not values read by the effect body — `scrollToMessageImperative` reads the DOM, and we need the effect to re-run each time the message list or virtualized rendered range changes so a target spliced into older history (or windowed out of the DOM) gets centered once its row commits.
  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetIdRef.current = null;
      releasePinnedCenter();
      return;
    }
    if (
      anchorRef.current.kind === "pinned-center" &&
      anchorRef.current.messageId !== targetMessageId
    ) {
      releasePinnedCenter();
    }
    if (handledTargetIdRef.current === targetMessageId || isLoading) return;
    if (!hasInitializedRef.current) return; // initial-mount path will handle.

    void virtualizerRenderVersion;
    // `pending` (virtualizer mid-jump) and `missing` (row not spliced in yet)
    // both leave the target unhandled; the next `messages` or rendered-range
    // commit re-runs this effect and retries until the row is centered.
    const result = scrollToMessageImperative(targetMessageId, {
      highlight: highlightTargetMessage,
    });
    if (result === "centered") {
      if (targetRetryRafRef.current !== null) {
        cancelAnimationFrame(targetRetryRafRef.current);
        targetRetryRafRef.current = null;
      }
      handledTargetIdRef.current = targetMessageId;
      onTargetReached?.(targetMessageId);
    } else if (result === "pending" && targetRetryRafRef.current === null) {
      // Virtua can finish correcting measured row offsets without changing its
      // rendered range. Retry on the next frame so completion observes the
      // final geometry rather than depending on an unrelated React render.
      targetRetryRafRef.current = requestAnimationFrame(() => {
        targetRetryRafRef.current = null;
        setTargetRetryVersion((version) => version + 1);
      });
    }
  }, [
    highlightTargetMessage,
    isLoading,
    messages,
    onTargetReached,
    releasePinnedCenter,
    scrollToMessageImperative,
    targetMessageId,
    targetRetryVersion,
    virtualizerRenderVersion,
  ]);

  React.useEffect(() => {
    return () => {
      if (highlightTimeoutRef.current !== null) {
        window.clearTimeout(highlightTimeoutRef.current);
      }
      if (programmaticScrollRafRef.current !== null) {
        cancelAnimationFrame(programmaticScrollRafRef.current);
      }
      if (targetSettleRafRef.current !== null) {
        cancelAnimationFrame(targetSettleRafRef.current);
      }
      if (targetRetryRafRef.current !== null) {
        cancelAnimationFrame(targetRetryRafRef.current);
      }
      if (targetCorrectionRafRef.current !== null) {
        cancelAnimationFrame(targetCorrectionRafRef.current);
      }
    };
  }, []);

  const onVirtualizerAtBottomStateChange = React.useCallback(
    (atBottom: boolean) => {
      if (!virtualizerOwnsPrependAnchoring) return;
      virtualizerAtBottomRef.current = atBottom;
      if (atBottom) {
        anchorRef.current = { kind: "at-bottom" };
        setNewMessageCount(0);
      }
      setIsAtBottom(atBottom);
    },
    [virtualizerOwnsPrependAnchoring],
  );

  return {
    onScroll,
    isAtBottom,
    newMessageCount,
    highlightedMessageId,
    scrollToBottom: scrollToBottomImperative,
    settleAtBottomAfterLayout,
    scrollToBottomOnNextUpdate,
    scrollToMessage: scrollToMessageImperative,
    onVirtualizerAtBottomStateChange,
  };
}
