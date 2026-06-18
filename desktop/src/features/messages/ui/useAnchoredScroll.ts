import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";

/**
 * Distance (in CSS pixels) below which we consider the scroll position
 * "at the bottom" of the message list. Tight enough that the user has to
 * actually scroll down to re-pin; permissive enough to tolerate sub-pixel
 * rounding from the layout engine.
 */
const AT_BOTTOM_THRESHOLD_PX = 32;

type AnchorState =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number };

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
  messages: TimelineMessage[];
  /** When set, scroll to and highlight this message on mount and on change. */
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
  /** Optional convergence fallback for a `targetMessageId` whose row is not in
   *  the DOM (windowed out of a virtualized list). When the DOM lookup fails,
   *  the hook delegates to this instead of waiting for a later commit that may
   *  never render the row. The consumer drives the virtualizer to the target,
   *  warming up if it's still being fetched, and fires `onTargetReached` itself
   *  on settle. Returns `true` once it owns the target (the hook marks it
   *  handled, no further dispatch). Absent (thread panel) → DOM-only retry. */
  convergeToTarget?: (messageId: string) => boolean;
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
  /** Single-writer scroll restore for the load-older index path. Sets
   *  `scrollTop` directly (no scroll event fires for a programmatic write),
   *  then re-seats the anchor + at-bottom bookkeeping so the next passive
   *  restore and `isAtBottom` read agree with where we put the scroll. */
  restoreScrollPosition: (scrollTop: number) => void;
};

function isAtBottomNow(container: HTMLDivElement) {
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

/**
 * Find the rendered message id that is closest in chronological order to
 * the anchor, scanning forward in `messages`. Used as the fallback when the
 * anchor's row is gone post-render (e.g. message deleted).
 */
function findNearestNewerMessageId(
  container: HTMLDivElement,
  messages: TimelineMessage[],
  anchorId: string,
): string | null {
  const anchorIndex = messages.findIndex((m) => m.id === anchorId);
  if (anchorIndex < 0) return null;

  for (let i = anchorIndex + 1; i < messages.length; i++) {
    const candidate = messages[i];
    const el = container.querySelector(`[data-message-id="${candidate.id}"]`);
    if (el) return candidate.id;
  }
  return null;
}

/**
 * Restore a message-kind anchor's on-screen offset after a layout shift.
 *
 * Finds the anchor row (or the nearest newer rendered row if the anchor
 * itself was removed), measures its current top-relative offset, and
 * `scrollBy(0, delta)` if the offset has drifted. Returns the new anchor
 * state the caller should write back:
 * - `{ kind: "message", ... }` — anchor (or its fallback) is in the DOM
 *   and now sits at its previous offset.
 * - `{ kind: "at-bottom" }` — anchor and all newer rendered rows are gone;
 *   caller should pin to the bottom and update at-bottom state.
 *
 * `scrollBy` is intentional over `scrollTop = ...`: relative adjustment
 * composes with the browser's own scroll anchoring and doesn't fight a
 * smooth-scroll in flight. Same rationale as the layout-effect restore.
 *
 * Used by both the post-commit layout effect (prepend / append / spinner
 * toggle / etc.) and the ResizeObserver (in-viewport reflow from image
 * decode, embed expansion, font load). Keeping them on one primitive
 * preserves the single-owner invariant of the hook.
 */
function restoreAnchorToMessage(
  container: HTMLDivElement,
  messages: TimelineMessage[],
  anchor: Extract<AnchorState, { kind: "message" }>,
): AnchorState {
  let anchorEl = container.querySelector<HTMLElement>(
    `[data-message-id="${anchor.messageId}"]`,
  );
  let usedAnchor: AnchorState = anchor;
  if (!anchorEl) {
    const fallbackId = findNearestNewerMessageId(
      container,
      messages,
      anchor.messageId,
    );
    if (fallbackId) {
      anchorEl = container.querySelector<HTMLElement>(
        `[data-message-id="${fallbackId}"]`,
      );
      if (anchorEl) {
        usedAnchor = {
          kind: "message",
          messageId: fallbackId,
          topOffset: anchor.topOffset,
        };
      }
    }
  }

  if (!anchorEl) {
    // Anchor message and all subsequent rendered messages are gone.
    container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
    return { kind: "at-bottom" };
  }

  const containerTop = container.getBoundingClientRect().top;
  const currentTop = anchorEl.getBoundingClientRect().top - containerTop;
  const delta = currentTop - usedAnchor.topOffset;
  if (Math.abs(delta) > 0.5) {
    container.scrollBy(0, delta);
  }
  return usedAnchor;
}

export function useAnchoredScroll({
  scrollContainerRef,
  contentRef,
  channelId,
  isLoading,
  messages,
  targetMessageId = null,
  onTargetReached,
  convergeToTarget,
}: UseAnchoredScrollOptions): UseAnchoredScrollResult {
  // Anchor lives in a ref because it must survive renders and is updated
  // both on scroll (commit-time read) and in the layout effect (post-render
  // restoration). useState would force re-renders we don't want.
  const anchorRef = React.useRef<AnchorState>({ kind: "at-bottom" });
  // Latest `messages` mirrored to a ref so the ResizeObserver effect can read
  // the current list without re-subscribing the observer on every commit
  // (which would also drop any in-flight resize callbacks). Kept fresh by a
  // layout effect below so the read is consistent with what's in the DOM.
  const messagesRef = React.useRef<TimelineMessage[]>(messages);
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [newMessageCount, setNewMessageCount] = React.useState(0);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);

  const hasInitializedRef = React.useRef(false);
  // Mirror the convergence fallback into a ref so the target effects read the
  // live callback without re-subscribing on every consumer render.
  const convergeToTargetRef = React.useRef(convergeToTarget);
  convergeToTargetRef.current = convergeToTarget;
  const prevLastMessageIdRef = React.useRef<string | undefined>(undefined);
  // Tracks the FRONT (oldest) rendered id so the restore effect can detect a
  // load-older prepend (front changed, tail unchanged) and cede it to the
  // index path — see IMPORTANT #1 in the restore effect below.
  const prevFirstMessageIdRef = React.useRef<string | undefined>(undefined);
  const prevMessageCountRef = React.useRef(0);
  const handledTargetIdRef = React.useRef<string | null>(null);
  // Set while a convergence loop owns the scroll position (jump-to-message into
  // windowed-out history). The library's reconcile loop is the sole writer
  // during convergence, so the anchored restore below must cede — otherwise its
  // at-bottom `scrollTo` would yank the view back as the target row splices in,
  // the same two-writer contention the prepend bail prevents. Cleared when the
  // target settles (consumer clears the route param → `targetMessageId` null).
  const convergingTargetIdRef = React.useRef<string | null>(null);
  const highlightTimeoutRef = React.useRef<number | null>(null);
  // One-shot: the consumer calls `scrollToBottomOnNextUpdate()` right before
  // it sends a message (see ChannelPane). When the user's own message then
  // appends, we snap to bottom even if they had scrolled up to read history.
  // Consumed (and cleared) by the next append in the restoration effect.
  const forceBottomOnNextAppendRef = React.useRef(false);

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
    handledTargetIdRef.current = null;
    convergingTargetIdRef.current = null;
    forceBottomOnNextAppendRef.current = false;
    if (highlightTimeoutRef.current !== null) {
      window.clearTimeout(highlightTimeoutRef.current);
      highlightTimeoutRef.current = null;
    }
  }, [channelId]);

  const scrollToBottomImperative = React.useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const container = scrollContainerRef.current;
      if (!container) return;
      anchorRef.current = { kind: "at-bottom" };
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

      el.scrollIntoView({
        block: "center",
        behavior: options.behavior ?? "auto",
      });

      // `scrollIntoView({ behavior: "smooth" })` starts an async animation, so
      // measuring after the call can still return the pre-animation position.
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

  // Re-seat the anchor + at-bottom bookkeeping after a programmatic scrollTop
  // write. A programmatic write fires no scroll event, so `onScroll` won't run
  // to refresh `anchorRef`/`isAtBottom` — we run the same derivation here so
  // the next passive restore and at-bottom read agree with the new position.
  // We deliberately do NOT touch `newMessageCount`: a load-older restore keeps
  // the reader mid-history, so the unread-count affordance must be untouched.
  const syncAnchorAfterProgrammaticScroll = React.useCallback(
    (container: HTMLDivElement) => {
      anchorRef.current = computeAnchor(container);
      const atBottom = anchorRef.current.kind === "at-bottom";
      setIsAtBottom((prev) => (prev === atBottom ? prev : atBottom));
    },
    [],
  );

  // Single-writer restore for the load-older index path (IMPORTANT #2). The
  // index path resolves the exact target `scrollTop` off the virtualizer's
  // settled measurement cache (`getOffsetForIndex`), so a bare assignment is
  // correct on the first write — no rAF re-assert loop, no manager scroll-state
  // machine. Re-seating the anchor afterwards keeps this the sole owner.
  const restoreScrollPosition = React.useCallback(
    (scrollTop: number) => {
      const container = scrollContainerRef.current;
      if (!container) return;
      container.scrollTop = scrollTop;
      syncAnchorAfterProgrammaticScroll(container);
    },
    [scrollContainerRef, syncAnchorAfterProgrammaticScroll],
  );

  // Scroll handler: recompute anchor + bottom state from the current
  // scroll position. Cheap enough to run on every scroll event — a single
  // `getBoundingClientRect` walk plus rect reads.
  const onScroll = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    anchorRef.current = computeAnchor(container);
    const atBottom = anchorRef.current.kind === "at-bottom";
    setIsAtBottom((prev) => (prev === atBottom ? prev : atBottom));
    if (atBottom) {
      setNewMessageCount(0);
    }
  }, [scrollContainerRef]);

  // ---------------------------------------------------------------------------
  // Anchor restoration: after every render, if the anchor was a message,
  // realign so that message sits at the same top-relative offset it had
  // before the render. This keeps scroll stable across appends, image loads,
  // and embed expansions. Load-older prepends are NOT handled here — they are
  // ceded to `useLoadOlderOnScroll`'s index anchor (see the prepend bail
  // below) so a single writer owns `scrollTop` on the prepend commit.
  // ---------------------------------------------------------------------------
  React.useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    // Mirror the current messages list into the ref read by the
    // ResizeObserver's restore path. Must happen before any early return so
    // a non-React layout shift sees the same array the next restoration
    // would use.
    messagesRef.current = messages;

    // First render after a reset (channel switch or initial mount): jump
    // to the requested target message, or to the bottom by default.
    if (!hasInitializedRef.current) {
      if (isLoading) return;
      if (targetMessageId) {
        // A cold deep-link target may not be in the DOM on this first
        // commit — the route screen fetches it by id and splices it in a
        // render or two later. If centering fails now, leave the timeline at
        // its default position and let the post-mount target effect (keyed on
        // `messages`) retry once the row lands, rather than marking it handled.
        if (scrollToMessageImperative(targetMessageId, { highlight: true })) {
          handledTargetIdRef.current = targetMessageId;
          // Consumers clear the route target (`messageId` URL param) on this
          // callback. The post-mount target effect below also fires it, but
          // for a target already in the DOM on first commit that effect bails
          // (handled ref is set), so the initial path must fire it too — else
          // the param sticks and re-clicking the same deep link is a no-op.
          onTargetReached?.(targetMessageId);
        } else {
          scrollToBottomImperative("auto");
        }
      } else {
        scrollToBottomImperative("auto");
      }
      hasInitializedRef.current = true;
      prevLastMessageIdRef.current = messages[messages.length - 1]?.id;
      prevFirstMessageIdRef.current = messages[0]?.id;
      prevMessageCountRef.current = messages.length;
      return;
    }

    const anchor = anchorRef.current;
    const lastMessage = messages[messages.length - 1];
    const firstMessage = messages[0];
    const prevLastId = prevLastMessageIdRef.current;
    const prevFirstId = prevFirstMessageIdRef.current;
    const prevCount = prevMessageCountRef.current;
    const newLatestArrived =
      lastMessage !== undefined && lastMessage.id !== prevLastId;
    // A convergence loop owns the scroll position while jumping to a windowed-out
    // target (its library reconcile is the sole writer). Cede every restore
    // branch to it — an at-bottom `scrollTo` here would chase the view back to
    // the bottom as the target's neighbours splice into the window mid-converge.
    // Refresh the tracked refs so the first post-settle commit isn't misread as
    // a prepend/append. Cleared when the target settles (targetMessageId null).
    if (convergingTargetIdRef.current !== null) {
      prevLastMessageIdRef.current = lastMessage?.id;
      prevFirstMessageIdRef.current = firstMessage?.id;
      prevMessageCountRef.current = messages.length;
      return;
    }
    // A load-older prepend grows the list at the FRONT while the tail is
    // unchanged. `useLoadOlderOnScroll` owns the prepend restore via its index
    // anchor (the single `scrollTop` writer). If this restore effect also ran
    // its anchored `scrollBy` on the same commit, two writers would fight over
    // `scrollTop`. So cede the prepend to the index path: refresh the tracked
    // refs and bail before the anchored branch. (Append and in-window reflow
    // leave the front id unchanged, so they fall through as before.)
    const isPrepend =
      firstMessage !== undefined &&
      prevFirstId !== undefined &&
      firstMessage.id !== prevFirstId &&
      !newLatestArrived;
    if (isPrepend) {
      prevLastMessageIdRef.current = lastMessage?.id;
      prevFirstMessageIdRef.current = firstMessage.id;
      prevMessageCountRef.current = messages.length;
      return;
    }

    // One-shot: an outbound send armed `scrollToBottomOnNextUpdate`. When the
    // resulting append lands, snap to bottom regardless of the current anchor,
    // then clear the flag. Bail before the anchored branch so the user's own
    // message pulls the view down.
    if (newLatestArrived && forceBottomOnNextAppendRef.current) {
      forceBottomOnNextAppendRef.current = false;
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      anchorRef.current = { kind: "at-bottom" };
      setIsAtBottom(true);
      setNewMessageCount(0);
      prevLastMessageIdRef.current = lastMessage?.id;
      prevFirstMessageIdRef.current = firstMessage?.id;
      prevMessageCountRef.current = messages.length;
      return;
    }

    if (anchor.kind === "at-bottom") {
      // Stick to bottom. Use scrollTo to avoid relying on scroll anchoring.
      container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
      if (newLatestArrived) setNewMessageCount(0);
    } else {
      // Anchored to a specific message. The shared helper finds it (with a
      // nearest-newer fallback if the row was removed) and corrects the
      // offset via `scrollBy`. If both the anchor and all newer rendered
      // rows are gone, it pins to the bottom and returns `at-bottom`.
      const restored = restoreAnchorToMessage(container, messages, anchor);
      anchorRef.current = restored;
      if (restored.kind === "at-bottom") {
        setIsAtBottom(true);
      }

      if (newLatestArrived) {
        const added = Math.max(1, messages.length - prevCount);
        setNewMessageCount((current) => current + added);
      }
    }

    prevLastMessageIdRef.current = lastMessage?.id;
    prevFirstMessageIdRef.current = firstMessage?.id;
    prevMessageCountRef.current = messages.length;
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
  // Content resize: when fonts load late, an image decodes, an embed expands,
  // or any in-viewport reflow happens that React isn't driving (so the
  // layout-effect doesn't fire), the anchor row's on-screen offset drifts.
  //
  // When stuck-to-bottom we re-pin to bottom. When anchored to a message we
  // call the same restore primitive the layout effect uses, so an in-viewport
  // reflow above the reader's eye shifts back into place. Without this,
  // anything that resizes without changing `messages` (link-card decode,
  // async embed expand, late font load, markdown that expands) silently
  // pushes the reading row around.
  // ---------------------------------------------------------------------------
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const container = scrollContainerRef.current;
      if (!container) return;
      // Cede entirely while a convergence loop owns scroll (jump to a
      // windowed-out target). Mid-jump the anchor is transiently `at-bottom`
      // — `computeAnchor` finds no crossing row until the virtualizer renders
      // rows at the new offset — so an unconditional re-pin here would yank
      // the in-flight jump down to the floor as rows measure. The convergence
      // loop is the sole writer until it settles (mirrors the layout effect's
      // `convergingTargetIdRef` bail).
      if (convergingTargetIdRef.current !== null) return;
      const anchor = anchorRef.current;
      if (anchor.kind === "at-bottom") {
        // Stuck to bottom: re-pin to the new floor. Virtualizer measurement
        // grows `scrollHeight` after the initial pin (rows below the fold
        // measure a frame or two late) without any `messages` change to drive
        // the layout effect, so this observer is the only thing that keeps the
        // view glued to the bottom as content settles.
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
        return;
      }
      // Use the same restore primitive as the layout effect so the
      // single-owner model holds across non-React-driven layout shifts.
      const restored = restoreAnchorToMessage(
        container,
        messagesRef.current,
        anchor,
      );
      anchorRef.current = restored;
      if (restored.kind === "at-bottom") {
        setIsAtBottom(true);
      }
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [contentRef, scrollContainerRef]);

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
      convergingTargetIdRef.current = null;
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
      // Row not in the DOM. In a virtualized list it may be windowed out and
      // never render from a passive commit, so delegate to the convergence
      // fallback: it drives the virtualizer to the target (warming up if a
      // deep-link target is still being fetched in) and, on settle, centers +
      // highlights it and fires `onTargetReached`. We mark the target handled
      // here so this effect stops re-dispatching, but deliberately do NOT fire
      // `onTargetReached` yet — clearing the route param now would cancel the
      // in-flight target fetch the loop is waiting on. Without a fallback
      // (thread panel), leave the target for a later `messages` commit.
      const converge = convergeToTargetRef.current;
      if (converge?.(targetMessageId)) {
        handledTargetIdRef.current = targetMessageId;
        convergingTargetIdRef.current = targetMessageId;
      }
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
    restoreScrollPosition,
  };
}
