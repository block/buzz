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

/**
 * Snapshot the reader's position for realization compensation: the first row
 * FULLY inside the viewport (its top at/below the scroller top) and that row's
 * top relative to the scroller top.
 *
 * Why *fully* visible and not the top-crossing straddler `computeAnchor` picks:
 * the CSS scroll-anchoring spec descends past partially-visible candidates and
 * anchors on the first fully-visible element for exactly the case we hit —
 * during an upscroll the realizing band is the freshly exposed content hanging
 * above the fold, so a straddler anchor sits *inside* that churning band and
 * can't measure the shift below it. The first fully-visible row sits one notch
 * below the churn, so re-pinning it to its saved offset cancels the net height
 * change of everything above it (the layout engine sums those deltas for us —
 * a row resizing below the anchor doesn't move the anchor's top, so it's
 * excluded for free). Returns null when nothing is fully visible.
 */
function snapshotReadingAnchor(
  container: HTMLDivElement,
): { id: string; topOffset: number } | null {
  const containerTop = container.getBoundingClientRect().top;
  const rows = container.querySelectorAll<HTMLElement>("[data-message-id]");
  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    if (rect.top >= containerTop) {
      const id = row.dataset.messageId;
      if (id) return { id, topOffset: rect.top - containerTop };
    }
  }
  return null;
}

/**
 * The height the browser is CURRENTLY using to lay out `row`. For a
 * `content-visibility: auto` row that has never painted this is its
 * `contain-intrinsic-size` reserve (the estimate) rather than its realized
 * height; for a row the browser has already realized-and-remembered (the `auto`
 * keyword) it is that remembered size. We read the computed
 * `contain-intrinsic-block-size` — Blink returns it as `"<n>px"` or
 * `"auto <n>px"` — and fall back to the live box height if the property is
 * empty/unsupported (e.g. WKWebView returning an empty string). Seeding the
 * resize map with this value is what turns a realization into a MEASURABLE
 * `realized - reserve` delta instead of an unmeasurable first sighting.
 */
function reservedRowHeight(row: HTMLElement): number {
  const raw = getComputedStyle(row).containIntrinsicBlockSize;
  const match = raw.match(/(-?\d+(?:\.\d+)?)px/);
  if (match) return Number.parseFloat(match[1]);
  return row.getBoundingClientRect().height;
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
  // ignores transient gaps and keeps chasing the floor. A `ref`, not state — the
  // guard runs on a native scroll event, outside React's render cycle.
  const settlingRef = React.useRef(false);
  // Baseline for realization/reflow compensation: the first row fully inside
  // the viewport (top at/below the scroller top) and its top offset, captured
  // on every scroll event and after every correction. The ResizeObserver
  // re-pins THIS row to THIS offset — a single measured pin that absorbs the
  // net height change of everything above it (see `snapshotReadingAnchor`).
  const readingAnchorRef = React.useRef<{
    id: string;
    topOffset: number;
  } | null>(null);

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
    // Baseline the realization-compensation anchor from THIS scroll snapshot —
    // RO fires after layout, so the "before" position can only come from the
    // last scroll event (or a prior correction), never from inside the RO
    // callback. Missing/stale baseline => the RO path skips rather than drifts.
    readingAnchorRef.current = snapshotReadingAnchor(container);
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
    isLoading,
    messages,
    onTargetReached,
    scrollContainerRef,
    scrollToBottomImperative,
    scrollToMessageImperative,
    targetMessageId,
  ]);

  // ---------------------------------------------------------------------------
  // Content resize: a height change React isn't driving — a bottom-pinned
  // in-viewport reflow (image decode, embed expand, late font), OR an
  // off-screen row above the reading position realizing to its true height as
  // the user scrolls up into it (content-visibility skip -> visible). Both grow
  // `scrollHeight` without a `messages` change, so the layout effect doesn't
  // fire. The ResizeObserver callback runs in the rendering steps AFTER layout
  // and BEFORE paint, which makes a `scrollBy` here same-frame invisible — this
  // is the correct trigger for compensation, not the async-dispatched
  // `contentvisibilityautostatechange` event (which may fire after the shifted
  // frame has already painted).
  //
  //   - at-bottom: re-pin to the new floor to stay glued.
  //   - mid-history: `overflow-anchor: none` is set on the scroller and the
  //     shipped WKWebView has no native anchoring anyway, so nothing holds the
  //     reading row across the reflow/realization — our writer must. This is
  //     the fix for the up-scroll jitter AND the latent reflow bug (both were
  //     previously left to a native anchoring that does not run here).
  //
  // Freshness (single-writer + no-fighting-the-wheel): `anchorRef.current` is
  // re-baselined by `onScroll` every scroll event, and scroll events are
  // dispatched earlier in the same frame's rendering steps than ResizeObserver
  // delivery — so when a realization RO fires mid-gesture, the anchor's saved
  // offset already reflects the user's current scroll position, and the drift
  // we measure is purely the layout shift above the reading row, not the user's
  // own wheel delta. We skip while settling a programmatic bottom pin so this
  // never races the floor-chase in `onScroll`.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: `messages` is an intentional re-sync trigger — on each committed render we (re)observe any newly-mounted `.timeline-row-cv` rows so a row appended by a load-older page starts being watched. The callback reads only stable refs; `channelId` forces a full re-subscribe when the keyed scroll container remounts.
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    // Last-known laid-out height per observed row. The compensable delta of a
    // resize is `newHeight - lastHeight`. We SEED each row at observe time with
    // the height the browser is currently using for layout — for a
    // `content-visibility: auto` row that has never painted, that is its
    // `contain-intrinsic-size` reserve (the estimate), NOT its realized height
    // (which the row does not report until it realizes). Seeding with the
    // reserve is what makes the very first RO delivery — the realization itself
    // — yield the true `realized - reserve` delta instead of being silently
    // swallowed as an unmeasurable first sighting.
    const lastHeights = new WeakMap<Element, number>();
    const observer = new ResizeObserver((entries) => {
      const container = scrollContainerRef.current;
      if (!container) return;
      // A programmatic bottom pin is still settling; `onScroll` owns the
      // floor-chase, so stay out of its way and don't double-write.
      if (settlingRef.current) return;
      if (anchorRef.current.kind === "at-bottom") {
        // Bottom-glue: a row realizing/reflowing while pinned grows the content,
        // so re-pin to the new floor to stay glued, and refresh the height map
        // so we don't treat this growth as a mid-history delta later.
        for (const entry of entries) {
          lastHeights.set(
            entry.target,
            entry.target.getBoundingClientRect().height,
          );
        }
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
        return;
      }
      // Mid-history: a batch of rows realized/reflowed this frame. The
      // reading anchor — the first fully-visible row, snapshotted by the last
      // scroll event — has shifted by the net height change of everything above
      // it. Re-pin it to its saved offset: a single measured correction (the
      // layout engine already summed the above-anchor deltas into the row's
      // top, and rows resizing below the anchor don't move it, so they're
      // excluded for free). This is the single scroll writer for realization;
      // `overflow-anchor: none` (and WKWebView's absence of it) mean nothing
      // else competes. We use the RO batch only as the TRIGGER — we detect that
      // a row's laid-out height actually changed (seeded from the reserve so a
      // first-realization delivery counts, not swallowed as a first sighting),
      // then correct from the anchor's own measured drift, never from summing
      // per-row deltas.
      let changed = false;
      for (const entry of entries) {
        const row = entry.target as HTMLElement;
        const height = row.getBoundingClientRect().height;
        const last = lastHeights.get(row);
        lastHeights.set(row, height);
        if (last === undefined) continue; // never seeded (defensive).
        if (Math.abs(height - last) > 0.5) changed = true;
      }
      if (!changed) return;
      const baseline = readingAnchorRef.current;
      if (!baseline) return; // no stable pre-batch snapshot: skip, don't drift.
      const anchorRow = container.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(baseline.id)}"]`,
      );
      if (!anchorRow) return;
      const containerTop = container.getBoundingClientRect().top;
      const currentTopOffset =
        anchorRow.getBoundingClientRect().top - containerTop;
      const drift = currentTopOffset - baseline.topOffset;
      if (Math.abs(drift) > 0.5) {
        container.scrollBy(0, drift);
        // Re-baseline after the correction so the next RO batch measures drift
        // from where we just pinned, not the pre-correction position.
        readingAnchorRef.current = snapshotReadingAnchor(container);
      }
    });
    // Observe every timeline row (not the content wrapper): a
    // `content-visibility: auto` row realizing to its true height is a resize
    // of THAT row's box but does not reliably fire a ResizeObserver on the
    // wrapper (Blink does not surface CV realization as an ancestor resize).
    // The RO callback runs after layout, before paint, so the compensating
    // `scrollBy` is same-frame invisible.
    for (const row of content.querySelectorAll<HTMLElement>(
      ".timeline-row-cv",
    )) {
      lastHeights.set(row, reservedRowHeight(row));
      observer.observe(row);
    }
    return () => observer.disconnect();
  }, [channelId, contentRef, scrollContainerRef, messages]);

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
