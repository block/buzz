import * as React from "react";

import {
  classifyTimelineMessageDelta,
  type TimelineMessageDelta,
} from "@/features/messages/lib/timelineSnapshot";
import {
  getTargetScrollPlacement,
  makeTargetKey,
  resolveTargetDivider,
  type ScrollTargetAlignment,
} from "@/features/messages/ui/anchoredScrollTarget";

/**
 * Distance (in CSS pixels) below which we consider the scroll position
 * "at the bottom" of the message list. Tight enough that the user has to
 * actually scroll down to re-pin; permissive enough to tolerate sub-pixel
 * rounding from the layout engine.
 */
const AT_BOTTOM_THRESHOLD_PX = 32;
// Pinned affordances need the physical floor, not the looser UI threshold.
const TRUE_BOTTOM_THRESHOLD_PX = 1;
type AnchorState =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number }
  | { kind: "pinned-center"; messageId: string; contentTop: number };

export function getPinnedCenterDrift({
  contentTop,
  currentContentTop,
}: {
  contentTop: number;
  currentContentTop: number;
}): number | null {
  const drift = currentContentTop - contentTop;
  return Math.abs(drift) > 0.5 ? drift : null;
}

export function shouldIgnorePinnedCenterScroll({
  currentScrollTop,
  expectedScrollTop,
  isWritingScroll,
}: {
  currentScrollTop: number;
  expectedScrollTop: number | null;
  isWritingScroll: boolean;
}): boolean {
  return isWritingScroll || expectedScrollTop === currentScrollTop;
}

type BottomSettleContainer = Pick<
  HTMLDivElement,
  "scrollHeight" | "clientHeight" | "scrollTop" | "scrollTo"
>;

export function settleProgrammaticBottomPin(
  container: BottomSettleContainer,
): boolean {
  container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
  return isAtTrueBottom(container);
}

export function shouldSettleForSplitPanel({
  isAtBottom,
  splitPanelOpen,
}: {
  isAtBottom: boolean;
  splitPanelOpen: boolean;
}): boolean {
  return isAtBottom && splitPanelOpen;
}

export function shouldSettleVirtualizedBottom({
  isAtBottom,
  messageDelta,
  messagesArrived,
  messagesChanged,
}: {
  isAtBottom: boolean;
  messageDelta: TimelineMessageDelta;
  messagesArrived: number;
  messagesChanged: boolean;
}): boolean {
  return (
    isAtBottom &&
    messageDelta !== "prepend" &&
    (messagesArrived > 0 || messagesChanged)
  );
}

type UseAnchoredScrollOptions = {
  /** Scroll container. Owned by the parent so external refs still compose. */
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  /** Inner content element — must wrap every renderable row, including the
   *  sentinel and bottom anchor. Used to schedule layout work on resize. */
  contentRef: React.RefObject<HTMLDivElement | null>;
  /** Resets when changed; lets us drop anchor + scroll state across channels. */
  channelId?: string | null;
  /** Suppresses initial scroll-to-bottom while a skeleton is showing. */
  isLoading: boolean;
  /** Source of truth for the rendered list. Used to detect new-at-bottom
   *  arrivals and to seed/refresh the anchor pre-render. */
  messages: Array<{ id: string }>;
  splitPanelOpen?: boolean;

  /** When set, scroll to this message on mount and on change. */
  targetMessageId?: string | null;
  /** Thread-open anchors use top/bottom; explicit targets default to center. */
  targetAlignment?: ScrollTargetAlignment;
  /** Whether a targeted message should pulse after scrolling to it. */
  highlightTargetMessage?: boolean;
  /** Keeps a targeted message centered until the user deliberately scrolls. */
  pinTargetCentered?: boolean;
  onTargetReached?: (messageId: string) => void;
  virtualScrollToMessage?: (
    messageId: string,
    options?: { behavior?: ScrollBehavior },
  ) => boolean;
  /** Imperative virtualizer-owned bottom jump, used only when virtualizer mode is active. */
  virtualScrollToBottom?: (behavior?: ScrollBehavior) => void;
  virtualSettleAtBottom?: () => void;
  /** When active, the virtualizer owns prepend compensation and bottom-state synchronization. */
  virtualizerOwnsPrependAnchoring?: boolean;
  /** Bumps when a virtualized range changes, so pending target/search retries can re-check newly mounted DOM. */
  virtualizerRenderVersion?: number;
};

type UseAnchoredScrollResult = {
  /** Pass through to the scroll container's `onScroll`. */
  onScroll: (
    event?: Pick<React.UIEvent<HTMLDivElement>, "currentTarget">,
  ) => void;
  /** True when the user is within `AT_BOTTOM_THRESHOLD_PX` of the bottom. */
  isAtBottom: boolean;
  /** Number of new messages that have arrived while the user is not at the
   *  bottom. Cleared when the user returns to the bottom. */
  newMessageCount: number;
  /** Message id that should pulse a highlight (target/active-search). */
  highlightedMessageId: string | null;
  /** Imperative: scroll to bottom. */
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  /** Arm a one-shot scroll-to-bottom that fires on the next appended message
   *  (used by the composer's send flow). */
  scrollToBottomOnNextUpdate: () => void;
  /** Imperative: scroll a specific message into view; optionally pulse it.
   *  Returns true if the row was found and scrolled, false otherwise. */
  scrollToMessage: (
    messageId: string,
    options?: { highlight?: boolean; behavior?: ScrollBehavior },
  ) => boolean;
  /** Syncs the hook's bottom affordances from a virtualizer-owned scroller. */
  onVirtualizerAtBottomStateChange: (atBottom: boolean) => void;
};

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

function isAtTrueBottom(
  container: Pick<
    HTMLDivElement,
    "scrollHeight" | "clientHeight" | "scrollTop"
  >,
) {
  return (
    container.scrollHeight - container.clientHeight - container.scrollTop <=
    TRUE_BOTTOM_THRESHOLD_PX
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
  targetAlignment = "center",
  highlightTargetMessage = true,
  pinTargetCentered = false,
  onTargetReached,
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
  const handledTargetRequestRef = React.useRef<string | null>(null);
  const targetRequestKey = makeTargetKey(targetMessageId, targetAlignment);
  const highlightTimeoutRef = React.useRef<number | null>(null);
  // Tracks a pending mount pin so channel switches can cancel it.
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
  // Preserve a deliberate anchor across its own programmatic scroll event.
  const programmaticScrollTopRef = React.useRef<number | null>(null);
  const isWritingScrollRef = React.useRef(false);
  const programmaticScrollRafRef = React.useRef<number | null>(null);

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
    handledTargetRequestRef.current = null;
    forceBottomOnNextAppendRef.current = false;
    settlingRef.current = false;
    programmaticScrollTopRef.current = null;
    isWritingScrollRef.current = false;
    if (programmaticScrollRafRef.current !== null) {
      cancelAnimationFrame(programmaticScrollRafRef.current);
      programmaticScrollRafRef.current = null;
    }
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
      // Programmatic events are delivered before the next frame. Expire the
      // exact destination then so a later non-pinned scroll is never swallowed.
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
    ): boolean => {
      const container = scrollContainerRef.current;
      if (!container) return false;
      const el = container.querySelector<HTMLElement>(
        `[data-message-id="${messageId}"]`,
      );
      if (virtualizerOwnsPrependAnchoring && virtualScrollToMessage) {
        if (el) {
          const rect = el.getBoundingClientRect();
          const containerRect = container.getBoundingClientRect();
          const isInViewport =
            rect.top >= containerRect.top &&
            rect.bottom <= containerRect.bottom;
          if (!isInViewport) {
            if (!virtualScrollToMessage(messageId, { behavior: "auto" })) {
              return false;
            }
            anchorRef.current = { kind: "message", messageId, topOffset: 0 };
            setIsAtBottom(false);
            return false;
          }
          const centeredTop = (container.clientHeight - rect.height) / 2;
          container.scrollTo({
            top: Math.max(
              0,
              container.scrollTop +
                (rect.top - containerRect.top) -
                centeredTop,
            ),
            behavior: options.behavior ?? "auto",
          });
        } else if (
          !virtualScrollToMessage(messageId, {
            behavior: options.behavior ?? "auto",
          })
        ) {
          return false;
        }
        anchorRef.current = { kind: "message", messageId, topOffset: 0 };
        setIsAtBottom(false);
        if (el && options.highlight && targetAlignment !== "bottom") {
          highlightMessage(messageId);
        }
        return el !== null;
      }

      if (!el) return false;

      const targetDivider = resolveTargetDivider({
        alignment: targetAlignment,
        container,
        firstMessageId: messages[0]?.id,
        targetMessageId: messageId,
      });
      if (targetDivider === undefined) return false;

      // A thread-open target can arrive one render after the mount fallback
      // queued its settling bottom pin. Once the target row is real, that
      // stale frame and its scroll-event settle guard must not overwrite a
      // non-bottom placement. A bottom target keeps the guard so it can chase
      // late row measurements to the physical floor.
      if (mountPinRafIdRef.current !== null) {
        cancelAnimationFrame(mountPinRafIdRef.current);
        mountPinRafIdRef.current = null;
      }
      settlingRef.current = targetAlignment === "bottom";

      const rect = el.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      const { contentTop, maxScrollTop, targetScrollTop, targetTopOffset } =
        getTargetScrollPlacement({
          alignment: targetAlignment,
          containerClientHeight: container.clientHeight,
          containerScrollHeight: container.scrollHeight,
          containerScrollTop: container.scrollTop,
          containerTop: containerRect.top,
          dividerTop: targetDivider?.getBoundingClientRect().top ?? null,
          targetHeight: rect.height,
          targetTop: rect.top,
        });

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
        const scrollTopBefore = container.scrollTop;
        container.scrollTo({
          top: targetScrollTop,
          behavior: options.behavior ?? "auto",
        });
        noteProgrammaticScroll(container, scrollTopBefore);

        // Smooth scrolling starts an async animation, so measuring after the call can still return the pre-animation position.
        // Save the clamped destination offset instead; otherwise a concurrent
        // render/ResizeObserver restore can fight the smooth scroll back toward
        // where it started.
        anchorRef.current =
          targetAlignment === "bottom"
            ? { kind: "at-bottom" }
            : {
                kind: "message",
                messageId,
                topOffset: targetTopOffset,
              };
      }
      if (!pinTargetCentered) {
        setIsAtBottom(maxScrollTop - targetScrollTop <= AT_BOTTOM_THRESHOLD_PX);
      }

      if (options.highlight && targetAlignment !== "bottom") {
        highlightMessage(messageId);
      }
      return true;
    },
    [
      highlightMessage,
      messages,
      noteProgrammaticScroll,
      pinTargetCentered,
      scrollContainerRef,
      targetAlignment,
      virtualizerOwnsPrependAnchoring,
      writePinnedCenterScroll,
      virtualScrollToMessage,
    ],
  );

  // Recompute anchor + bottom state from the current scroll position.
  const onScroll = React.useCallback(
    (event?: Pick<React.UIEvent<HTMLDivElement>, "currentTarget">) => {
      const container = scrollContainerRef.current;
      if (!container) return;
      // Ignore a native scroll queued for a keyed-out conversation surface.
      if (event && event.currentTarget !== container) return;
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
      if (
        anchorRef.current.kind !== "at-bottom" &&
        shouldIgnorePinnedCenterScroll({
          currentScrollTop: container.scrollTop,
          expectedScrollTop: programmaticScrollTopRef.current,
          isWritingScroll: isWritingScrollRef.current,
        })
      ) {
        // Chromium can emit duplicate events for one programmatic write; keep
        // its destination armed until noteProgrammaticScroll's rAF expires it.
        return;
      }
      if (anchorRef.current.kind === "pinned-center") {
        // Explicit input releases the pin; bare native scroll events are
        // ambiguous because programmatic events can arrive after long reflow.
        return;
      }
      anchorRef.current = computeAnchor(container);
      const atBottom = anchorRef.current.kind === "at-bottom";
      setIsAtBottom((prev) => (prev === atBottom ? prev : atBottom));
      if (atBottom) {
        setNewMessageCount(0);
      }
    },
    [scrollContainerRef, virtualizerOwnsPrependAnchoring],
  );

  // ---------------------------------------------------------------------------
  // Anchor restoration: after every render, stick to the bottom if the user is
  // there. The reading position across prepend / in-viewport reflow is held by
  // the browser's native scroll anchoring (overflow-anchor) now that every
  // loaded row stays in the DOM, so there is no JS message-anchor restore.
  // ---------------------------------------------------------------------------

  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId must rerun initialization after the reset effect even when cached messages and target are referentially unchanged.
  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    // First render after a reset (channel switch or initial mount): jump
    // to the requested target message, or to the bottom by default.
    if (!hasInitializedRef.current) {
      if (isLoading) return;
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
        // A cold deep-link target may not be in the DOM on this first
        // commit — the route screen fetches it by id and splices it in a
        // render or two later. If centering fails now, leave the timeline at
        // its default position and let the post-mount target effect (keyed on
        // `messages`) retry once the row lands, rather than marking it handled.
        if (
          scrollToMessageImperative(targetMessageId, {
            highlight: highlightTargetMessage,
          })
        ) {
          handledTargetRequestRef.current = targetRequestKey;
          onTargetReached?.(targetMessageId);
        } else {
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
    channelId,
    highlightTargetMessage,
    isLoading,
    messages,
    onTargetReached,
    scrollContainerRef,
    scrollToBottomImperative,
    scrollToMessageImperative,
    targetMessageId,
    targetRequestKey,
    repinPinnedCenter,
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
      if (anchorRef.current.kind === "pinned-center") {
        repinPinnedCenter();
      } else if (
        anchorRef.current.kind === "at-bottom" &&
        !virtualizerOwnsPrependAnchoring
      ) {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      }
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [
    channelId,
    contentRef,
    repinPinnedCenter,
    scrollContainerRef,
    virtualizerOwnsPrependAnchoring,
  ]);

  // Release pinned centers only when the reader deliberately takes control.
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId deliberately re-subscribes after a keyed or conditional scroll-container mount replaces ref.current.
  React.useEffect(() => {
    if (!pinTargetCentered) return;
    const container = scrollContainerRef.current;
    if (!container) return;

    const handleUserInteraction = () => releasePinnedCenter();
    const handlePointerDown = (event: PointerEvent) => {
      if (event.target === container) releasePinnedCenter();
    };
    container.addEventListener("wheel", handleUserInteraction, {
      passive: true,
    });
    container.addEventListener("touchstart", handleUserInteraction, {
      passive: true,
    });
    container.addEventListener("pointerdown", handlePointerDown, {
      passive: true,
    });
    container.addEventListener("keydown", handleUserInteraction);
    return () => {
      container.removeEventListener("wheel", handleUserInteraction);
      container.removeEventListener("touchstart", handleUserInteraction);
      container.removeEventListener("pointerdown", handlePointerDown);
      container.removeEventListener("keydown", handleUserInteraction);
    };
  }, [channelId, pinTargetCentered, releasePinnedCenter, scrollContainerRef]);

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
  // biome-ignore lint/correctness/useExhaustiveDependencies: `messages` and `virtualizerRenderVersion` are intentional retry triggers, not values read by the effect body — the effect reads the DOM (querySelector), and we need it to re-run each time the message list or virtualized rendered range changes so a target spliced into older history gets centered once its row commits.
  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetRequestRef.current = null;
      releasePinnedCenter();
      return;
    }
    if (
      anchorRef.current.kind === "pinned-center" &&
      anchorRef.current.messageId !== targetMessageId
    ) {
      releasePinnedCenter();
    }
    if (handledTargetRequestRef.current === targetRequestKey || isLoading)
      return;
    if (!hasInitializedRef.current) return; // initial-mount path will handle.

    void virtualizerRenderVersion;
    const container = scrollContainerRef.current;
    if (!container) return;
    const el = container.querySelector<HTMLElement>(
      `[data-message-id="${targetMessageId}"]`,
    );
    if (!el && virtualizerOwnsPrependAnchoring) {
      if (
        scrollToMessageImperative(targetMessageId, {
          highlight: highlightTargetMessage,
        })
      ) {
        handledTargetRequestRef.current = targetRequestKey;
        onTargetReached?.(targetMessageId);
      }
      return;
    }
    if (!el) {
      // Row not in the DOM yet. A cold deep-link target is fetched by id and
      // spliced into `messages` a render or two later; this effect re-runs on
      // each `messages` commit and retries until the row exists.
      return;
    }
    if (
      scrollToMessageImperative(targetMessageId, {
        highlight: highlightTargetMessage,
      })
    ) {
      handledTargetRequestRef.current = targetRequestKey;
      onTargetReached?.(targetMessageId);
    }
  }, [
    highlightTargetMessage,
    isLoading,
    messages,
    onTargetReached,
    releasePinnedCenter,
    scrollContainerRef,
    scrollToMessageImperative,
    targetMessageId,
    targetRequestKey,
    virtualizerOwnsPrependAnchoring,
    virtualizerRenderVersion,
  ]);

  // Adjacent target chrome can commit after its row without changing messages
  // or box size. Retry unresolved targets on child-list commits too.
  React.useEffect(() => {
    if (
      !targetMessageId ||
      handledTargetRequestRef.current === targetRequestKey ||
      isLoading ||
      typeof MutationObserver === "undefined"
    ) {
      return;
    }
    const content = contentRef.current;
    if (!content) return;

    const observer = new MutationObserver(() => {
      if (handledTargetRequestRef.current === targetRequestKey) return;
      if (
        scrollToMessageImperative(targetMessageId, {
          highlight: highlightTargetMessage,
        })
      ) {
        handledTargetRequestRef.current = targetRequestKey;
        onTargetReached?.(targetMessageId);
        observer.disconnect();
      }
    });
    observer.observe(content, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [
    contentRef,
    highlightTargetMessage,
    isLoading,
    onTargetReached,
    scrollToMessageImperative,
    targetMessageId,
    targetRequestKey,
  ]);

  React.useEffect(() => {
    return () => {
      if (highlightTimeoutRef.current !== null) {
        window.clearTimeout(highlightTimeoutRef.current);
      }
      if (programmaticScrollRafRef.current !== null) {
        cancelAnimationFrame(programmaticScrollRafRef.current);
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
    scrollToBottomOnNextUpdate,
    scrollToMessage: scrollToMessageImperative,
    onVirtualizerAtBottomStateChange,
  };
}
