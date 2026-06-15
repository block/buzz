import * as React from "react";

/**
 * Anchor-based scroll preservation for a chat-shaped message list.
 *
 * Design (adapted from element-hq/matrix-react-sdk `ScrollPanel.tsx`,
 * cross-validated against Zulip's `message_viewport.ts`):
 *
 * - Scroll state is one of:
 *   - `{ kind: "atBottom" }` — sticky, new messages scroll us with them.
 *   - `{ kind: "anchored", messageId, topOffset }` — the message with `id`
 *     should sit at `topOffset` pixels below the viewport top.
 *
 * - The state is the *source of truth*. The DOM's `scrollTop` is a *derived
 *   value* recomputed once per render commit in a layout effect.
 *
 * - This eliminates the multiple-writers-racing-rAF-loops failure mode that
 *   the previous `useTimelineScrollManager` + `useLoadOlderOnScroll` +
 *   `ResizeObserver` combination suffered from when older history prepended.
 *
 * Why anchor by **top** (not Matrix's bottom): Buzz users scrolling up to read
 * history have their attention near the *top* of the viewport. When something
 * below the anchor reflows (a video card mounting, an embed expanding), the
 * user's eye doesn't move. Matrix uses bottomOffset because their primary
 * mode is sitting near the bottom; ours is reading.
 *
 * Why we leave CSS `overflow-anchor: auto` enabled on the container: it
 * handles mutations our algorithm doesn't see (image loads, late media
 * expansion). The layout-effect runs *after* the browser's anchor adjustment
 * in the same frame and writes the final scrollTop, so there's no race.
 */

type AnchorState =
  | { kind: "atBottom" }
  | { kind: "anchored"; messageId: string; topOffset: number };

type UseAnchoredScrollOptions = {
  /**
   * Reset to `{ atBottom: true }` whenever this key changes. Use the channel
   * id (or thread root id) so navigating to a new conversation always starts
   * pinned to the latest message.
   */
  resetKey: string | null | undefined;
  /**
   * Sequence of currently-rendered messages, oldest → newest. The hook reads
   * its length and the id of the last message to detect appends; it does not
   * touch any other field.
   */
  messageIds: readonly string[];
  /**
   * Async callback to fetch older history. Invoked when the user nears the
   * top sentinel and `hasOlderMessages` is true. The anchor algorithm holds
   * scroll position automatically; this callback must *not* touch scroll.
   */
  fetchOlder?: () => Promise<void> | void;
  /** Whether more older history exists. Disables the top-sentinel trigger. */
  hasOlderMessages?: boolean;
  /**
   * Initial-load gate. While true, the hook does not run its first
   * scroll-to-bottom or anchor-restore (avoids fighting skeleton mounts).
   */
  isLoading?: boolean;
  /**
   * One-shot: if non-null and the hook hasn't handled it yet, scroll this
   * message id into the centre of the viewport and switch state to
   * `anchored`. `onTargetReached` fires once the message is in place.
   */
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
  /**
   * Optional mutable ref mirrored from this hook's `isAtBottom` state on
   * every change. Lets sibling DOM-writing effects (e.g. composer padding)
   * gate their behaviour on the SAME at-bottom truth the hook computes,
   * instead of duplicating thresholds. Single decision-maker, single
   * `AT_BOTTOM_THRESHOLD_PX`.
   */
  atBottomRef?: React.MutableRefObject<boolean>;
};

/** How close to the bottom counts as "at the bottom". Avoids subpixel misses. */
const AT_BOTTOM_THRESHOLD_PX = 24;

/** IntersectionObserver margin for the older-history sentinel. */
const FETCH_OLDER_ROOT_MARGIN = "400px 0px 0px 0px";

export type AnchoredScrollHandle = {
  /** Programmatically scroll to the latest message. */
  scrollToBottom: (behavior?: ScrollBehavior) => void;
  /** Number of messages appended while not at the bottom (resets on stick). */
  newMessageCount: number;
  /** Whether the user is currently pinned to the bottom. */
  isAtBottom: boolean;
  /** Ref to attach to the scroll container. */
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  /** Ref to attach to a top sentinel (1px tall, above all messages). */
  topSentinelRef: React.RefObject<HTMLDivElement | null>;
  /** Highlight currently active (for target-message flash). */
  highlightedMessageId: string | null;
};

export function useAnchoredScroll({
  resetKey,
  messageIds,
  fetchOlder,
  hasOlderMessages = false,
  isLoading = false,
  targetMessageId = null,
  onTargetReached,
  atBottomRef,
}: UseAnchoredScrollOptions): AnchoredScrollHandle {
  const scrollContainerRef = React.useRef<HTMLDivElement | null>(null);
  const topSentinelRef = React.useRef<HTMLDivElement | null>(null);

  // The single source of truth for where the viewport should sit.
  const anchorRef = React.useRef<AnchorState>({ kind: "atBottom" });
  // Set true after the initial scroll-to-bottom runs once for this resetKey.
  const initializedRef = React.useRef(false);

  // Public reactive state — these drive UI affordances (jump-to-latest pill,
  // unread badge). Mutating refs alone wouldn't re-render the consumer.
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [newMessageCount, setNewMessageCount] = React.useState(0);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);

  // Mirror at-bottom into an optional external ref so sibling DOM effects
  // (composer padding) can read the same truth without duplicating the
  // threshold.
  React.useEffect(() => {
    if (atBottomRef) atBottomRef.current = isAtBottom;
  }, [atBottomRef, isAtBottom]);

  // Track previous tail to distinguish "new message appended" from a prepend
  // (length grew but lastId unchanged) or unrelated rerender.
  const previousLastIdRef = React.useRef<string | null>(null);
  const handledTargetRef = React.useRef<string | null>(null);

  // ---- reset when resetKey changes -----------------------------------------

  // biome-ignore lint/correctness/useExhaustiveDependencies: resetKey is the sole trigger
  React.useLayoutEffect(() => {
    anchorRef.current = { kind: "atBottom" };
    initializedRef.current = false;
    previousLastIdRef.current = null;
    handledTargetRef.current = null;
    setIsAtBottom(true);
    setNewMessageCount(0);
    setHighlightedMessageId(null);
  }, [resetKey]);

  // ---- anchor walk ---------------------------------------------------------

  /**
   * Recompute the anchor from the live DOM. Walks message elements top-down
   * and picks the first one whose bottom edge has crossed the viewport top —
   * i.e. the topmost row the reader is actually looking at. Records its
   * `rect.top` relative to the container (which is negative when the row
   * straddles the top edge). If the user is at the bottom, switches to
   * `{ atBottom: true }` instead.
   *
   * Choice of anchor matches Buzz's UX: a user scrolled up to read history
   * has their eyes at the *top* of the viewport, so the pixel that must stay
   * stable under reflow is `topRow.rect.top - container.rect.top`. Matrix
   * uses the bottom-most row + `bottomOffset` because chat readers sit near
   * the bottom; that pairing drifts in our case when content *inside* the
   * viewport reflows (image-load between viewport top and a low anchor).
   */
  const recomputeAnchor = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const distanceFromBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    if (distanceFromBottom <= AT_BOTTOM_THRESHOLD_PX) {
      anchorRef.current = { kind: "atBottom" };
      setIsAtBottom((cur) => (cur ? cur : true));
      setNewMessageCount(0);
      return;
    }

    const containerTop = container.getBoundingClientRect().top;
    const messageEls =
      container.querySelectorAll<HTMLElement>("[data-message-id]");

    let candidate: HTMLElement | null = null;
    for (const el of messageEls) {
      const rect = el.getBoundingClientRect();
      // First row whose bottom edge sits below the viewport top — i.e. the
      // topmost row crossing or below the top edge. Top-down so we pick the
      // first crossing, not the last.
      if (rect.bottom > containerTop) {
        candidate = el;
        break;
      }
    }

    if (!candidate) {
      // Every rendered message is above the viewport (atypical: would mean
      // the user is past the last message, which the distance-from-bottom
      // check above should already have caught). Treat as at-bottom.
      anchorRef.current = { kind: "atBottom" };
      setIsAtBottom((cur) => (cur ? cur : true));
      return;
    }

    const messageId = candidate.dataset.messageId;
    if (!messageId) return;

    anchorRef.current = {
      kind: "anchored",
      messageId,
      topOffset: candidate.getBoundingClientRect().top - containerTop,
    };
    setIsAtBottom((cur) => (cur ? false : cur));
  }, []);

  // ---- the one writer: post-commit scroll restoration ----------------------

  const lastId =
    messageIds.length > 0 ? messageIds[messageIds.length - 1] : null;

  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || isLoading) return;

    // First commit after load (or after resetKey change): jump to bottom
    // unless a target message will be scrolled into view by its own effect.
    if (!initializedRef.current) {
      initializedRef.current = true;
      previousLastIdRef.current = lastId;
      if (!targetMessageId) {
        container.scrollTop = container.scrollHeight;
        anchorRef.current = { kind: "atBottom" };
        setIsAtBottom(true);
      }
      return;
    }

    const previousLastId = previousLastIdRef.current;
    const hasAppended = lastId !== null && lastId !== previousLastId;
    previousLastIdRef.current = lastId;

    const anchor = anchorRef.current;

    if (anchor.kind === "atBottom") {
      container.scrollTop = container.scrollHeight - container.clientHeight;
      if (hasAppended) setNewMessageCount(0);
      return;
    }

    // Anchored: re-pin the captured message to its captured topOffset.
    let anchorEl = container.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
    );

    if (!anchorEl) {
      anchorEl = findFallbackAnchor(container, anchor.messageId, messageIds);
      if (!anchorEl) {
        container.scrollTop = container.scrollHeight - container.clientHeight;
        anchorRef.current = { kind: "atBottom" };
        setIsAtBottom(true);
        return;
      }
      // Preserve topOffset so the visual position stays put on the fallback.
      anchorRef.current = {
        kind: "anchored",
        messageId: anchorEl.dataset.messageId as string,
        topOffset: anchor.topOffset,
      };
    }

    const containerTop = container.getBoundingClientRect().top;
    const currentTop = anchorEl.getBoundingClientRect().top - containerTop;
    const delta = currentTop - anchor.topOffset;
    // Half-pixel deadband: swallow subpixel font reflow. The captured
    // topOffset is preserved across layout-effect restorations (only
    // re-captured on user scroll or explicit target-message), so a swallowed
    // delta doesn't compound — the next restoration sees the same oldTop.
    // We write `scrollTop += delta` rather than `scrollBy(0, delta)` because
    // both are programmatic (neither composes with overflow-anchor — the
    // browser only auto-adjusts in response to DOM mutations, not script
    // writes). Direct assignment avoids the smooth-scroll behaviour
    // `scrollBy` can pick up from CSS `scroll-behavior` and reads more
    // obviously as "additive nudge to restore captured anchor."
    if (Math.abs(delta) > 0.5) {
      container.scrollTop += delta;
    }

    if (hasAppended) {
      setNewMessageCount((cur) => cur + 1);
    }
  }, [isLoading, lastId, messageIds, targetMessageId]);

  // ---- fetch-older trigger (IntersectionObserver on top sentinel) ----------

  const fetchOlderRef = React.useRef(fetchOlder);
  React.useEffect(() => {
    fetchOlderRef.current = fetchOlder;
  });

  React.useEffect(() => {
    const sentinel = topSentinelRef.current;
    const container = scrollContainerRef.current;
    if (
      !sentinel ||
      !container ||
      !hasOlderMessages ||
      isLoading ||
      !fetchOlder
    ) {
      return;
    }

    let inFlight = false;
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[0];
        if (!entry || !entry.isIntersecting || inFlight) return;
        inFlight = true;
        // Snapshot the anchor right before the fetch so the post-commit
        // restore has fresh data even if the user keeps scrolling during
        // the request.
        recomputeAnchor();
        const result = fetchOlderRef.current?.();
        Promise.resolve(result).finally(() => {
          // Don't clear `inFlight` here — let the next observer disconnect
          // (driven by the deps changing when more messages load) start a
          // fresh observation. This is the same shape as Matrix's
          // pendingFillRequests gate.
          inFlight = false;
        });
      },
      { root: container, rootMargin: FETCH_OLDER_ROOT_MARGIN },
    );

    observer.observe(sentinel);
    return () => {
      observer.disconnect();
    };
  }, [fetchOlder, hasOlderMessages, isLoading, recomputeAnchor]);

  // ---- target-message scroll-into-view -------------------------------------

  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetRef.current = null;
      setHighlightedMessageId(null);
      return;
    }
    if (handledTargetRef.current === targetMessageId || isLoading) return;

    const container = scrollContainerRef.current;
    if (!container) return;

    const target = container.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(targetMessageId)}"]`,
    );
    if (!target) return;

    handledTargetRef.current = targetMessageId;

    const containerTop = container.getBoundingClientRect().top;
    const targetRect = target.getBoundingClientRect();
    // Centre the target in the viewport.
    const desiredTop = container.clientHeight / 2 - targetRect.height / 2;
    const currentTop = targetRect.top - containerTop;
    container.scrollTop += currentTop - desiredTop;

    anchorRef.current = {
      kind: "anchored",
      messageId: targetMessageId,
      topOffset: desiredTop,
    };
    setIsAtBottom(false);
    setHighlightedMessageId(targetMessageId);
    onTargetReached?.(targetMessageId);

    const timeout = window.setTimeout(() => {
      setHighlightedMessageId((cur) => (cur === targetMessageId ? null : cur));
    }, 2_000);
    return () => {
      window.clearTimeout(timeout);
    };
  }, [isLoading, onTargetReached, targetMessageId, messageIds]);

  // ---- imperative scrollToBottom (for jump-to-latest button) ---------------

  const scrollToBottom = React.useCallback(
    (behavior: ScrollBehavior = "smooth") => {
      const container = scrollContainerRef.current;
      if (!container) return;
      container.scrollTo({
        top: container.scrollHeight - container.clientHeight,
        behavior,
      });
      anchorRef.current = { kind: "atBottom" };
      setIsAtBottom(true);
      setNewMessageCount(0);
    },
    [],
  );

  // ---- wire scroll listener onto the container -----------------------------
  // `recomputeAnchor` is the sole scroll-time work. It updates `anchorRef`
  // (and visible-state setters), and never writes scrollTop.

  React.useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    container.addEventListener("scroll", recomputeAnchor, { passive: true });
    return () => {
      container.removeEventListener("scroll", recomputeAnchor);
    };
  }, [recomputeAnchor]);

  return {
    scrollToBottom,
    newMessageCount,
    isAtBottom,
    scrollContainerRef,
    topSentinelRef,
    highlightedMessageId,
  };
}

/**
 * Find a replacement anchor when the original was removed from the DOM. We
 * prefer the nearest *newer* still-rendered message (the one immediately
 * after the removed id in `messageIds`), because anchoring forward keeps the
 * user's view stable as content below reflows. If none exists, the caller
 * should fall back to bottom-stick.
 */
function findFallbackAnchor(
  container: HTMLElement,
  removedId: string,
  messageIds: readonly string[],
): HTMLElement | null {
  const idx = messageIds.indexOf(removedId);
  if (idx === -1) {
    // We don't even know where the anchor was. Pick the topmost visible
    // message and let the caller re-pin to that.
    return container.querySelector<HTMLElement>("[data-message-id]");
  }
  for (let i = idx + 1; i < messageIds.length; i++) {
    const el = container.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(messageIds[i])}"]`,
    );
    if (el) return el;
  }
  return null;
}
