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

  // Track previous tail to distinguish "new message appended" from
  // "history prepended" from "unrelated re-render".
  const previousLastIdRef = React.useRef<string | null>(null);
  const previousLengthRef = React.useRef(0);
  const handledTargetRef = React.useRef<string | null>(null);

  // ---- reset when resetKey changes -----------------------------------------

  React.useLayoutEffect(() => {
    anchorRef.current = { kind: "atBottom" };
    initializedRef.current = false;
    previousLastIdRef.current = null;
    previousLengthRef.current = 0;
    handledTargetRef.current = null;
    setIsAtBottom(true);
    setNewMessageCount(0);
    setHighlightedMessageId(null);
  }, [resetKey]);

  // ---- anchor walk ---------------------------------------------------------

  /**
   * Recompute the anchor from the live DOM. Walks message elements bottom-up
   * and picks the topmost one still in the viewport, recording its top offset
   * inside the scroll container. If the user is at the bottom, switches to
   * `{ atBottom: true }` instead.
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
    const messageEls = container.querySelectorAll<HTMLElement>(
      "[data-message-id]",
    );

    // Bottom-up walk: find the topmost message whose top is still visible.
    // (Equivalently: the first message we hit walking from the bottom that
    // is at or above the viewport top. We pick the *next* one — the first
    // still inside the viewport.) Iterate from the end and keep updating the
    // candidate until we find one above the viewport.
    let candidate: HTMLElement | null = null;
    for (let i = messageEls.length - 1; i >= 0; i--) {
      const el = messageEls[i];
      const rect = el.getBoundingClientRect();
      if (rect.bottom <= containerTop) {
        // This message is above the viewport. The previous candidate
        // (one position later, still in viewport) is what we want.
        break;
      }
      candidate = el;
    }

    if (!candidate) {
      // Nothing in the viewport (e.g. mid-resize, empty list). Don't update.
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

  // ---- scroll handler (user-driven) ----------------------------------------

  // While the user is actively scrolling, we sample the anchor on every event
  // so the next render commit knows where to pin. This is the only place
  // `onScroll` writes; it never touches scrollTop itself.
  const onScroll = React.useCallback(() => {
    recomputeAnchor();
  }, [recomputeAnchor]);

  // ---- the one writer: post-commit scroll restoration ----------------------

  // Detect append vs prepend vs unchanged based on the messageIds sequence.
  const lastId = messageIds.length > 0 ? messageIds[messageIds.length - 1] : null;
  const length = messageIds.length;

  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    if (isLoading) {
      // Don't touch scrollTop during initial load — skeleton is mounting.
      return;
    }

    // First commit after load completes (or after resetKey changed): jump to
    // bottom, unless a target message is set.
    if (!initializedRef.current) {
      initializedRef.current = true;
      previousLastIdRef.current = lastId;
      previousLengthRef.current = length;

      if (!targetMessageId) {
        container.scrollTop = container.scrollHeight;
        anchorRef.current = { kind: "atBottom" };
        setIsAtBottom(true);
      }
      return;
    }

    const previousLastId = previousLastIdRef.current;
    const previousLength = previousLengthRef.current;
    const hasAppended = lastId !== null && lastId !== previousLastId;
    const lengthDelta = length - previousLength;

    previousLastIdRef.current = lastId;
    previousLengthRef.current = length;

    const anchor = anchorRef.current;

    if (anchor.kind === "atBottom") {
      // Stick to bottom: write scrollHeight - clientHeight.
      container.scrollTop = container.scrollHeight - container.clientHeight;
      if (hasAppended) {
        // Clear unread count — user is following live.
        setNewMessageCount(0);
      }
      return;
    }

    // Anchored: find the anchor message and pin its top to topOffset.
    let anchorEl = container.querySelector<HTMLElement>(
      `[data-message-id="${cssEscape(anchor.messageId)}"]`,
    );

    if (!anchorEl) {
      // Anchor message disappeared (moderation delete, eviction).
      // Fall back to nearest newer message; if none, snap to bottom.
      anchorEl = findFallbackAnchor(container, anchor.messageId, messageIds);
      if (!anchorEl) {
        container.scrollTop = container.scrollHeight - container.clientHeight;
        anchorRef.current = { kind: "atBottom" };
        setIsAtBottom(true);
        return;
      }
      // Update anchor id but keep the same topOffset so visual position
      // stays put on the fallback.
      anchorRef.current = {
        kind: "anchored",
        messageId: anchorEl.dataset.messageId ?? anchor.messageId,
        topOffset: anchor.topOffset,
      };
    }

    const containerTop = container.getBoundingClientRect().top;
    const currentTop = anchorEl.getBoundingClientRect().top - containerTop;
    const delta = currentTop - anchor.topOffset;
    if (delta !== 0) {
      container.scrollTop += delta;
    }

    if (hasAppended && lengthDelta > 0) {
      // User is reading history and a new message arrived — bump the count.
      setNewMessageCount((cur) => cur + Math.max(1, lengthDelta));
    }
  }, [isLoading, lastId, length, messageIds, targetMessageId]);

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
      `[data-message-id="${cssEscape(targetMessageId)}"]`,
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
      setHighlightedMessageId((cur) =>
        cur === targetMessageId ? null : cur,
      );
    }, 2_000);
    return () => {
      window.clearTimeout(timeout);
    };
  }, [isLoading, onTargetReached, targetMessageId]);

  // ---- imperative scrollToBottom (for jump-to-latest button) ---------------

  const scrollToBottom = React.useCallback((behavior: ScrollBehavior = "smooth") => {
    const container = scrollContainerRef.current;
    if (!container) return;
    container.scrollTo({
      top: container.scrollHeight - container.clientHeight,
      behavior,
    });
    anchorRef.current = { kind: "atBottom" };
    setIsAtBottom(true);
    setNewMessageCount(0);
  }, []);

  // ---- wire onScroll onto the container ------------------------------------

  React.useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    container.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      container.removeEventListener("scroll", onScroll);
    };
  }, [onScroll]);

  return {
    scrollToBottom,
    newMessageCount,
    isAtBottom,
    scrollContainerRef,
    topSentinelRef,
    highlightedMessageId,
  };
}

/** CSS.escape polyfill that falls back to the spec for older test runtimes. */
function cssEscape(value: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  return value.replace(/(["\\])/g, "\\$1");
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
      `[data-message-id="${cssEscape(messageIds[i])}"]`,
    );
    if (el) return el;
  }
  return null;
}
