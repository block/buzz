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
// Bottom-settle loop tuning. The loop watches the scroll position once per
// frame while a programmatic bottom jump settles: it must not fight a live
// smooth-scroll animation (instant corrective writes mid-animation make the
// motion jerky and, worse, the engine keeps animating toward its stale target
// anyway), so it only issues a corrective pin after the position has held
// still for `SETTLE_STABLE_FRAMES` frames while short of the true floor —
// i.e. after the animation has died. The hard cap bounds the loop when the
// floor never stabilizes (e.g. content that keeps reflowing indefinitely).
const SETTLE_STABLE_FRAMES = 2;
const SETTLE_MAX_MS = 2_000;

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
};

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
  // ignores transient gaps (the settle loop below owns all corrective writes).
  // A `ref`, not state — the guard runs on a native scroll event, outside
  // React's render cycle.
  const settlingRef = React.useRef(false);
  // rAF id for the settle loop, so a channel switch / re-arm can cancel it.
  const settleRafIdRef = React.useRef<number | null>(null);
  // Count of scroll events produced by our own anchor-holding writes (prepend
  // restore, resize re-pin) that `onScroll` must swallow WITHOUT re-capturing
  // the anchor. The write is re-pinning the saved anchor row; capturing the
  // post-write position would bake in drift the next write is about to correct
  // (rows prepended above the viewport realize their `content-visibility`
  // estimates a few frames after the restore, shrinking the content above the
  // reading row — the browser fires no scroll event for that shrink and native
  // scroll anchoring does not compensate above the viewport).
  const programmaticScrollEventsRef = React.useRef(0);

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
    if (settleRafIdRef.current !== null) {
      cancelAnimationFrame(settleRafIdRef.current);
      settleRafIdRef.current = null;
    }
    programmaticScrollEventsRef.current = 0;
    if (highlightTimeoutRef.current !== null) {
      window.clearTimeout(highlightTimeoutRef.current);
      highlightTimeoutRef.current = null;
    }
    if (mountPinRafIdRef.current !== null) {
      cancelAnimationFrame(mountPinRafIdRef.current);
      mountPinRafIdRef.current = null;
    }
    // Unmount cleanup: stop the settle loop so a dead component's rAF chain
    // doesn't keep scheduling against a detached container.
    return () => {
      settlingRef.current = false;
      if (settleRafIdRef.current !== null) {
        cancelAnimationFrame(settleRafIdRef.current);
        settleRafIdRef.current = null;
      }
    };
  }, [channelId]);

  // Arm the bottom-settle guard and start (or restart) the settle loop. A
  // programmatic bottom jump is not atomic: a smooth jump (scroll-to-latest
  // pill) computes its animation target from the scroll metrics at call time,
  // and `content-visibility` rows realizing their true heights as the
  // animation passes them make that target stale — the engine settles ABOVE
  // the real floor and then emits no further scroll events, so an event-driven
  // guard can never issue the final corrective write (the strand distance
  // scales with the root font size; the 1.1 default text zoom pushed it past
  // the e2e floor budget). The loop instead samples once per frame: while the
  // position is still moving it stays hands-off (fighting a live animation is
  // jerky and the engine re-targets anyway); once the position holds still
  // short of the true floor, the animation is dead and it issues an instant
  // corrective pin. Disarms at the true floor or at the hard time cap.
  const armBottomSettle = React.useCallback(() => {
    settlingRef.current = true;
    if (settleRafIdRef.current !== null) {
      cancelAnimationFrame(settleRafIdRef.current);
      settleRafIdRef.current = null;
    }
    const container = scrollContainerRef.current;
    const startedAt = performance.now();
    let lastTop = Number.NaN;
    let stableFrames = 0;
    // Real user input during the settle window means the user has taken over —
    // stop chasing the floor immediately instead of yanking them back down.
    // Only trusted input events disarm; the stale animation's scroll events
    // don't fire these.
    const abortOnUserInput = () => {
      settlingRef.current = false;
    };
    const inputEvents = ["wheel", "touchstart", "keydown"] as const;
    for (const type of inputEvents) {
      container?.addEventListener(type, abortOnUserInput, { passive: true });
    }
    const cleanup = () => {
      for (const type of inputEvents) {
        container?.removeEventListener(type, abortOnUserInput);
      }
    };
    const tick = () => {
      settleRafIdRef.current = null;
      const container = scrollContainerRef.current;
      if (!container || !settlingRef.current) {
        settlingRef.current = false;
        cleanup();
        return;
      }
      if (performance.now() - startedAt >= SETTLE_MAX_MS) {
        settlingRef.current = false;
        cleanup();
        return;
      }
      const top = container.scrollTop;
      if (top === lastTop) {
        stableFrames += 1;
      } else {
        stableFrames = 0;
        lastTop = top;
      }
      if (stableFrames >= SETTLE_STABLE_FRAMES) {
        // Stability alone isn't enough to disarm — the floor itself must be
        // confirmed while the position is holding still. A smooth animation
        // can touch the floor transiently and still walk backward on later
        // ticks, so disarming on a mid-flight floor reading would hand those
        // ticks back to `computeAnchor` as a phantom scroll-up.
        if (isAtTrueBottom(container)) {
          // Physically at the floor and no longer moving: settled. Later
          // growth (image decode, late rows) is the at-bottom ResizeObserver's
          // job, not the settle loop's.
          settlingRef.current = false;
          cleanup();
          return;
        }
        settleProgrammaticBottomPin(container);
        stableFrames = 0;
        lastTop = container.scrollTop;
      }
      settleRafIdRef.current = requestAnimationFrame(tick);
    };
    settleRafIdRef.current = requestAnimationFrame(tick);
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
      // until the settle loop confirms the true floor.
      armBottomSettle();
      container.scrollTo({ top: container.scrollHeight, behavior });
      setIsAtBottom(true);
      setNewMessageCount(0);
    },
    [armBottomSettle, scrollContainerRef],
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
    // Row measurement can grow `scrollHeight` after a bottom pin and emit scroll
    // events while `scrollTop` holds at the old floor — opening a transient gap
    // above the true bottom. `computeAnchor` would read that as a deliberate
    // scroll-up and latch a message anchor, freezing the view short of bottom.
    // While settling, hold the at-bottom anchor and swallow the event — the
    // rAF settle loop armed by `armBottomSettle` owns all corrective writes
    // (an event-driven guard can't finish the job: a stale smooth-scroll
    // animation strands the view above the floor and then emits no further
    // scroll events, so the final corrective pin must fire outside the event
    // stream).
    if (settlingRef.current) {
      return;
    }
    // A scroll event echoing one of our own anchor-holding writes: swallow it
    // without re-capturing, so the saved anchor offset keeps describing where
    // the reading row BELONGS. Re-capturing here would adopt drift the next
    // re-pin is about to correct. User scrolls are unaffected — only writes we
    // issued ourselves increment this counter.
    if (programmaticScrollEventsRef.current > 0) {
      programmaticScrollEventsRef.current -= 1;
      return;
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
      armBottomSettle();
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
      const row = container.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
      );
      if (row) {
        const currentTopOffset =
          row.getBoundingClientRect().top -
          container.getBoundingClientRect().top;
        const drift = currentTopOffset - anchor.topOffset;
        if (Math.abs(drift) > 0.5) {
          const scrollTopBefore = container.scrollTop;
          container.scrollBy(0, drift);
          // Only count the write if it actually moved (a clamped write emits
          // no scroll event, and a stale count would swallow a user scroll).
          if (container.scrollTop !== scrollTopBefore) {
            programmaticScrollEventsRef.current += 1;
          }
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
    armBottomSettle,
    isLoading,
    messages,
    onTargetReached,
    scrollContainerRef,
    scrollToBottomImperative,
    scrollToMessageImperative,
    targetMessageId,
  ]);

  // ---------------------------------------------------------------------------
  // Content resize: reflow that React isn't driving (image decode, embed
  // expand, late font load, `content-visibility` realizing a row's true height)
  // grows the content without a `messages` change, so the layout effect doesn't
  // fire. While stuck to the bottom, re-pin to the new floor to stay glued.
  // While anchored mid-history, native scroll anchoring (overflow-anchor) holds
  // the reading row for in-viewport reflow — but it has no anchor node ABOVE
  // the viewport near the top edge, so a just-prepended history page realizing
  // its `contain-intrinsic-size` estimates over the next frames drifts the
  // reading row (the drift scales with the root font size, so the 1.1 default
  // text zoom pushed it past the e2e settle budget). Re-pin the anchored row to
  // its saved offset here; when native anchoring already held it, the drift is
  // ~0 and this is a no-op.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is a deliberate re-subscription trigger — the effect body reads only the stable refs, but on a channel switch the keyed scroll container remounts and contentRef.current becomes a fresh node, so the observer must disconnect from the previous channel's detached node and re-observe the live one.
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const container = scrollContainerRef.current;
      if (!container) return;
      const anchor = anchorRef.current;
      if (anchor.kind === "at-bottom") {
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
        return;
      }
      const row = container.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
      );
      if (!row) return;
      const drift =
        row.getBoundingClientRect().top -
        container.getBoundingClientRect().top -
        anchor.topOffset;
      if (Math.abs(drift) > 0.5) {
        const scrollTopBefore = container.scrollTop;
        container.scrollBy(0, drift);
        // Only count the write if it actually moved (a clamped write emits no
        // scroll event, and a stale count would swallow a user scroll).
        if (container.scrollTop !== scrollTopBefore) {
          programmaticScrollEventsRef.current += 1;
        }
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
