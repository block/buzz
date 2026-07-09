import * as React from "react";

import { classifyTimelineMessageDelta } from "@/features/messages/lib/timelineSnapshot";

/**
 * Distance (in CSS pixels) below which we consider the scroll position
 * "at the bottom" of the message list. Tight enough that the user has to
 * actually scroll down to re-pin; permissive enough to tolerate sub-pixel
 * rounding from the layout engine.
 */
const AT_BOTTOM_THRESHOLD_PX = 32;
// Tests and user-visible "pinned" affordances need the view at the physical
// floor, not merely within the looser UI at-bottom threshold. The loose
// threshold decides whether the user is close enough to count as reading the
// latest message; this strict threshold decides when a programmatic bottom pin
// has actually finished settling.
const TRUE_BOTTOM_THRESHOLD_PX = 1;

type AnchorState =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number };

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

  /** When set, scroll to and highlight this message on mount and on change. */
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;

  /**
   * Opt-in, default-off polled settle gate for the mid-history prepend
   * re-anchor. When `enabled`, the prepend drift-correction (`scrollBy`) is
   * deferred while a user fling is in progress and applied only once the scroll
   * has quiesced — see {@link SettleGateOptions}. When absent/off, the timeline
   * behaves byte-identically to main: the correction runs immediately on the
   * prepend commit, as it always has.
   */
  settleGate?: SettleGateOptions;
};

/**
 * Configuration for the polled quiet-window settle gate.
 *
 * The gate exists because there is no event-driven "user fling has ended"
 * signal that survives WebKit's momentum-event coalescing — a debounce on
 * `scroll` events starves on WebKit exactly when a fling is fastest. So we
 * poll `scrollTop` on `requestAnimationFrame` and declare settle only after
 * `quietFrames` *consecutive* frames of zero movement.
 *
 * Design edge (#1): WebKit freezes scroll-position reads for ~2 coalesced
 * frames mid-fling. A gate that fires on the first still frame would read that
 * freeze as "settled" and re-anchor mid-momentum — recreating the walk-blind
 * jump this gate is meant to remove. `quietFrames` MUST exceed that freeze
 * window; the default of 3 clears the ~2-frame freeze with one frame of margin.
 * Tune against the classifier, not by eye.
 */
export type SettleGateOptions = {
  /** Master switch. Off (or option omitted) ⇒ zero behavior change vs. main. */
  enabled: boolean;
  /**
   * Consecutive zero-delta rAF frames required to declare the scroll settled.
   * Must exceed WebKit's coalesced-freeze window (~2). Defaults to
   * {@link DEFAULT_SETTLE_QUIET_FRAMES}.
   */
  quietFrames?: number;
};

/**
 * Default consecutive-still-frame count for the settle gate. Chosen to clear
 * WebKit's ~2-frame coalesced-freeze window (design edge #1) with a one-frame
 * margin, so the gate never mistakes the freeze for a genuine settle.
 */
export const DEFAULT_SETTLE_QUIET_FRAMES = 3;

/**
 * How recently a `scroll` event must have fired for the gate to treat the
 * scroll as *in live motion* at layout-effect time. Live momentum emits a
 * `scroll` event every frame (~16ms) continuously; a discrete one-shot
 * `scrollTop` write emits a single event and then goes silent. A window a few
 * frames wide cleanly separates "still fling" from "settled after a discrete
 * jump" without a deferred sample — which is the requirement: the still-path
 * must correct synchronously in the layout effect, paying zero painted frames
 * of displacement, exactly as main does (Dawn's ruling on
 * `timeline-no-shift.spec.ts:429`). A deferred rAF probe leaks one painted
 * frame at full prepend height; a timestamp read does not.
 */
export const SETTLE_MOTION_WINDOW_MS = 100;

type UseAnchoredScrollResult = {
  /** Pass through to the scroll container's `onScroll`. */
  onScroll: () => void;
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
 * Poll `container.scrollTop` on `requestAnimationFrame` and invoke `onSettle`
 * once the position has held still for `quietFrames` *consecutive* frames.
 *
 * This is the settle gate's core. It exists because WebKit coalesces momentum
 * `scroll` events, so an event-debounce never fires at the true end of a fling
 * (design context in {@link SettleGateOptions}). Polling the position directly
 * side-steps the event stream entirely: a fling is "over" when the number stops
 * changing, and the consecutive-frame requirement is what makes it robust to
 * WebKit's mid-fling ~2-frame position-read freeze — a single still frame is
 * not enough, so the freeze cannot be mistaken for a settle.
 *
 * Returns a `cancel` function; call it if the observation is superseded (e.g. a
 * newer prepend arrives, or the component unmounts) so the rAF loop stops and
 * `onSettle` never fires for stale work.
 */
export function observeScrollSettle(
  container: Pick<HTMLDivElement, "scrollTop">,
  quietFrames: number,
  onSettle: () => void,
  scheduleFrame: (cb: () => void) => number = requestAnimationFrame,
  cancelFrame: (id: number) => void = cancelAnimationFrame,
): () => void {
  // Guard: a non-positive frame count would settle instantly and defeat the
  // gate's purpose. Clamp to the minimum that still clears the freeze window.
  const required = Math.max(quietFrames, DEFAULT_SETTLE_QUIET_FRAMES);
  let lastTop = container.scrollTop;
  let stillFrames = 0;
  let rafId: number | null = null;
  let cancelled = false;

  const tick = () => {
    if (cancelled) return;
    const top = container.scrollTop;
    if (top === lastTop) {
      stillFrames += 1;
    } else {
      stillFrames = 0;
      lastTop = top;
    }
    if (stillFrames >= required) {
      rafId = null;
      onSettle();
      return;
    }
    rafId = scheduleFrame(tick);
  };

  rafId = scheduleFrame(tick);

  return () => {
    cancelled = true;
    if (rafId !== null) {
      cancelFrame(rafId);
      rafId = null;
    }
  };
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
function computeAnchor(container: HTMLDivElement): AnchorState {
  if (isAtBottomNow(container)) {
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

  targetMessageId = null,
  onTargetReached,
  settleGate,
}: UseAnchoredScrollOptions): UseAnchoredScrollResult {
  // Anchor lives in a ref because it must survive renders and is updated
  // both on scroll (commit-time read) and in the layout effect (post-render
  // restoration). useState would force re-renders we don't want.
  const anchorRef = React.useRef<AnchorState>({ kind: "at-bottom" });
  const [isAtBottom, setIsAtBottom] = React.useState(true);
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
  // Cancel handle for an in-flight settle observation armed by the prepend
  // re-anchor while a fling is in progress (settle gate only). Held so a newer
  // prepend, a channel switch, or unmount can supersede the pending correction
  // before its `onSettle` fires — otherwise a stale re-anchor could snap the
  // view after the user has already flung somewhere else.
  const settleObserverCancelRef = React.useRef<(() => void) | null>(null);
  // Timestamp (ms) of the most recent `scroll` event, written in `onScroll`.
  // The prepend re-anchor reads it synchronously at layout-effect time to
  // decide still-vs-moving without a deferred sample (see
  // `SETTLE_MOTION_WINDOW_MS`): a still scroll gets a synchronous, main-identical
  // re-anchor (no displaced frame paints); only a live fling arms the async
  // settle path.
  const lastScrollTsRef = React.useRef(0);
  // Keep the latest settle-gate config in a ref so the layout effect can read
  // it without adding it to the dependency array (it must not re-run the
  // restoration logic just because the caller passed a new options object).
  const settleGateRef = React.useRef(settleGate);
  settleGateRef.current = settleGate;

  // Reset everything when the channel changes — the layout effect that runs
  // immediately after this reset is responsible for either jumping to bottom
  // or to the target message for the new channel.
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is intentionally the sole trigger — we want this effect to fire exactly when the channel changes (and on mount).
  React.useLayoutEffect(() => {
    anchorRef.current = { kind: "at-bottom" };
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
    if (highlightTimeoutRef.current !== null) {
      window.clearTimeout(highlightTimeoutRef.current);
      highlightTimeoutRef.current = null;
    }
    if (mountPinRafIdRef.current !== null) {
      cancelAnimationFrame(mountPinRafIdRef.current);
      mountPinRafIdRef.current = null;
    }
    if (settleObserverCancelRef.current !== null) {
      settleObserverCancelRef.current();
      settleObserverCancelRef.current = null;
    }
  }, [channelId]);

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
      container.scrollTo({ top: container.scrollHeight, behavior });
      setIsAtBottom(true);
      setNewMessageCount(0);
    },
    [scrollContainerRef],
  );

  // Arm a one-shot: the next append snaps to bottom regardless of where the
  // user is. The consumer calls this right before sending so their own
  // outbound message pulls the view down even if they'd scrolled up.
  const scrollToBottomOnNextUpdate = React.useCallback(() => {
    forceBottomOnNextAppendRef.current = true;
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
      if (!el) return false;

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
      setIsAtBottom(maxScrollTop - targetScrollTop <= AT_BOTTOM_THRESHOLD_PX);

      if (options.highlight) {
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
      }
      return true;
    },
    [scrollContainerRef],
  );

  // Scroll handler: recompute anchor + bottom state from the current
  // scroll position. Cheap enough to run on every scroll event — a single
  // `getBoundingClientRect` walk plus rect reads.
  const onScroll = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    // Record when this scroll event fired so the prepend re-anchor can read
    // live-motion state synchronously at layout-effect time (settle gate).
    lastScrollTsRef.current =
      typeof performance !== "undefined" ? performance.now() : Date.now();
    // Row measurement can grow `scrollHeight` after a bottom pin and emit scroll
    // events while `scrollTop` holds at the old floor — opening a transient gap
    // above the true bottom. `computeAnchor` would read that as a deliberate
    // scroll-up and latch a message anchor, freezing the view short of bottom.
    // While settling, keep the anchor at-bottom and chase the physical floor.
    if (settlingRef.current) {
      if (settleProgrammaticBottomPin(container)) {
        settlingRef.current = false;
      } else {
        return;
      }
    }
    anchorRef.current = computeAnchor(container);
    const atBottom = anchorRef.current.kind === "at-bottom";
    setIsAtBottom((prev) => (prev === atBottom ? prev : atBottom));
    if (atBottom) {
      setNewMessageCount(0);
    }
  }, [scrollContainerRef]);

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
      // Defer the scroll out of the layout effect so the current paint commits
      // first; cancelled on channel switch via the reset effect's rAF guard.
      const pinToBottomOnMount = () => {
        anchorRef.current = { kind: "at-bottom" };
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
        if (scrollToMessageImperative(targetMessageId, { highlight: true })) {
          handledTargetIdRef.current = targetMessageId;
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
    const isPrepend =
      classifyTimelineMessageDelta({
        current: messages,
        previous: prevMessages,
      }) === "prepend";

    // One-shot: an outbound send armed `scrollToBottomOnNextUpdate`. When the
    // resulting append lands, snap to bottom regardless of the current anchor,
    // then clear the flag. Bail before the anchored branch so the user's own
    // message pulls the view down.
    if (newLatestArrived && forceBottomOnNextAppendRef.current) {
      forceBottomOnNextAppendRef.current = false;
      anchorRef.current = { kind: "at-bottom" };
      settlingRef.current = true;
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      setIsAtBottom(true);
      setNewMessageCount(0);
      prevLastMessageIdRef.current = lastMessage?.id;
      prevFirstMessageIdRef.current = firstMessage?.id;
      prevMessageCountRef.current = messages.length;
      prevMessagesRef.current = messages;
      return;
    }

    if (anchor.kind === "at-bottom") {
      // Stick to bottom across the append.
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      if (newLatestArrived) setNewMessageCount(0);
    } else if (messagesArrived > 0) {
      // Anchored mid-history. An older-history prepend grows the content above
      // the reading row; the browser's native scroll anchoring does NOT correct
      // this at the top edge (no anchor node above the viewport when scrollTop
      // is ~0), so re-pin the anchored row to its saved offset by id. This is
      // the single scroll writer for the prepend — the load-older observer only
      // triggers the fetch. We run it in this post-commit layout effect (not the
      // observer's promise callback) because the prepended rows commit on a
      // deferred snapshot a few frames later, so the row's true position is only
      // known here.
      //
      // Re-pin measured fresh at call time: find the anchored row, read its
      // current top offset, and `scrollBy` the drift back to its saved offset.
      // Deferred settle callers re-run this so they compensate the row's
      // *settle-time* position, not a stale mid-fling measurement.
      const applyReanchor = () => {
        const currentRow = container.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
        );
        if (!currentRow) return;
        const currentTopOffset =
          currentRow.getBoundingClientRect().top -
          container.getBoundingClientRect().top;
        const drift = currentTopOffset - anchor.topOffset;
        if (Math.abs(drift) > 0.5) {
          container.scrollBy(0, drift);
        }
      };

      const gate = settleGateRef.current;
      if (gate?.enabled) {
        // Settle gate on: the prepend re-anchor is the one mid-fling scroll
        // writer, and firing it against live user momentum is what produces the
        // walk-blind jump. But deferral is only worth its cost under live
        // momentum — when the reader is at rest there is no jerk to avoid, and
        // deferring the correction (even by a single rAF) lets one frame paint
        // at full prepend height, a visible jump the sampler catches (Dawn's
        // ruling on `timeline-no-shift.spec.ts:429`). So decide still-vs-moving
        // from a signal already known *here*, synchronously: the freshness of
        // the last `scroll` event. Live momentum keeps `scroll` firing every
        // frame; a discrete jump or a settled reader leaves the stamp stale.
        // The window is a few frames wide so WebKit's ~2-frame coalesced freeze
        // (the same physics the k≥3 clamp respects) cannot misread a live fling
        // as still and fire the synchronous correction into real momentum.
        //
        // Still ⇒ `applyReanchor()` inline, identical to main's synchronous
        // commit — no displaced frame ever paints. Moving ⇒ defer behind the
        // polled quiet window, superseding any correction still pending from a
        // prior prepend so only the newest anchor is re-pinned.
        //
        // A3 trade (stated in the PR body): at rest the correction is
        // synchronous and behaves exactly as main. Only under a live fling is
        // the reading row held at full prepend height until the fling ends,
        // then snapped back once at settle — confined to true flings, the only
        // window the gate was ever meant to touch.
        const now =
          typeof performance !== "undefined" ? performance.now() : Date.now();
        const moving = now - lastScrollTsRef.current < SETTLE_MOTION_WINDOW_MS;
        if (settleObserverCancelRef.current !== null) {
          settleObserverCancelRef.current();
          settleObserverCancelRef.current = null;
        }
        if (moving) {
          settleObserverCancelRef.current = observeScrollSettle(
            container,
            gate.quietFrames ?? DEFAULT_SETTLE_QUIET_FRAMES,
            () => {
              settleObserverCancelRef.current = null;
              applyReanchor();
            },
          );
        } else {
          applyReanchor();
        }
      } else {
        applyReanchor();
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
    isLoading,
    messages,
    onTargetReached,
    scrollContainerRef,
    scrollToBottomImperative,
    scrollToMessageImperative,
    targetMessageId,
  ]);

  // ---------------------------------------------------------------------------
  // Content resize: while stuck to the bottom, an in-viewport reflow (image
  // decode, embed expand, late font load) that React isn't driving grows
  // `scrollHeight` without a `messages` change, so the layout effect doesn't
  // fire — re-pin to the new floor here to stay glued. When anchored
  // mid-history, native scroll anchoring (overflow-anchor) holds the reading
  // row across the reflow, so there's nothing to do.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is a deliberate re-subscription trigger — the effect body reads only the stable refs, but on a channel switch the keyed scroll container remounts and contentRef.current becomes a fresh node, so the observer must disconnect from the previous channel's detached node and re-observe the live one.
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const container = scrollContainerRef.current;
      if (!container) return;
      if (anchorRef.current.kind === "at-bottom") {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      }
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [channelId, contentRef, scrollContainerRef]);

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
  // biome-ignore lint/correctness/useExhaustiveDependencies: `messages` is an intentional trigger, not a read — the effect reads the DOM (querySelector), and we need it to re-run each time the rendered row set changes so a target spliced into older history gets centered once its row commits.
  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetIdRef.current = null;
      return;
    }
    if (handledTargetIdRef.current === targetMessageId || isLoading) return;
    if (!hasInitializedRef.current) return; // initial-mount path will handle.

    const container = scrollContainerRef.current;
    if (!container) return;
    const el = container.querySelector<HTMLElement>(
      `[data-message-id="${targetMessageId}"]`,
    );
    if (!el) {
      // Row not in the DOM yet. A cold deep-link target is fetched by id and
      // spliced into `messages` a render or two later; this effect re-runs on
      // each `messages` commit and retries until the row exists.
      return;
    }
    handledTargetIdRef.current = targetMessageId;
    scrollToMessageImperative(targetMessageId, { highlight: true });
    onTargetReached?.(targetMessageId);
  }, [
    isLoading,
    messages,
    onTargetReached,
    scrollContainerRef,
    scrollToMessageImperative,
    targetMessageId,
  ]);

  React.useEffect(() => {
    return () => {
      if (highlightTimeoutRef.current !== null) {
        window.clearTimeout(highlightTimeoutRef.current);
      }
      if (settleObserverCancelRef.current !== null) {
        settleObserverCancelRef.current();
        settleObserverCancelRef.current = null;
      }
    };
  }, []);

  return {
    onScroll,
    isAtBottom,
    newMessageCount,
    highlightedMessageId,
    scrollToBottom: scrollToBottomImperative,
    scrollToBottomOnNextUpdate,
    scrollToMessage: scrollToMessageImperative,
  };
}
