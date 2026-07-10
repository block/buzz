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
// Realization compensation only runs when the reading anchor's captured scroll
// position is consistent with the frame the ResizeObserver fires in. Under
// WebKit async ("coordinated") scrolling the rAF baseline read and the RO
// post-layout read can straddle a compositor commit, so a fragment of the
// user's own momentum can leak into the measured shift. If the RENDERED scroll
// (the painted wheel motion this frame — see `applyMidHistoryCorrection`)
// exceeds this bound, momentum is clearly in flight and the two reads are not
// trustworthy together — we SKIP the correction rather than risk folding the
// wheel delta into the pin. We gate on rendered, not raw `scrollTop`, delta
// because WebKit coalesces momentum into `scrollTop` on its own clock: a raw
// delta can read large on a frame that painted still. Under-correcting a single
// realization is invisible (the next quiet frame catches it); fighting the
// wheel is the visible lurch. Chosen at roughly one frame of aggressive
// trackpad momentum; tune against the gate.
const COMPENSATION_SCROLL_SKIP_PX = 120;
// Distance below the scroller top at which the reading anchor is chosen. We
// pick the first row whose top sits at least this far below the viewport top
// rather than the first row past the top edge, so the anchor stays OUT of the
// freshly-exposed realization band hanging just under the fold during an
// upscroll. A row inside that band can re-measure to a garbage position when
// the ResizeObserver re-queries it mid-realization; a row a notch below the
// churn moves only by the net height change above it. Matches the probe's
// SAFE_MARGIN so the gate measures the same anchor the writer pins.
const READING_ANCHOR_SAFE_MARGIN_PX = 60;

// How far ABOVE the current viewport top the reflow-attribution walk
// (`sumAboveAnchorShift`) reaches. The realization/reflow that moves the anchor
// happens in the freshly-exposed band just above the fold; a row straddling the
// top edge still shifts the anchor when it realizes, so the band extends one
// generous row-height above `scrollTop`. Anything further up scrolled past long
// ago and does not move the anchor this frame — including it would sum stale
// de-realization drift AND make the walk O(channel) on the non-virtualized DOM.
const REFLOW_BAND_ABOVE_FOLD_PX = 250;

type AnchorState =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number };

/**
 * A pre-realization snapshot of the reading-anchor row: its id, viewport-
 * relative top offset, the scrollTop at capture, and a live handle to the row
 * element so a later observer can re-measure the SAME row post-layout without
 * a fresh `querySelector`. Written by the per-rAF sampler; read as the baseline
 * by both mid-history observers (rAF and RO).
 */
type ReadingAnchor = {
  id: string;
  topOffset: number;
  scrollTop: number;
  row: HTMLElement;
};

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
 * excluded for free). We require the row's top to sit at least
 * `READING_ANCHOR_SAFE_MARGIN_PX` below the scroller top so the anchor never
 * sits inside the realization band itself. Returns null when no such row
 * exists.
 *
 * We also capture `scrollTop` alongside the viewport-relative `topOffset` so
 * compensation can be computed scroll-invariantly: the row's document position
 * `scrollTop + topOffset` changes ONLY when content above it reflows — a user
 * scroll moves `scrollTop` and `topOffset` by equal-and-opposite amounts and
 * leaves the sum fixed. That decoupling is what makes the correction correct on
 * WebKit even when the baseline is one frame stale relative to the user's live
 * momentum: we compensate the reflow, never the wheel.
 */
function snapshotReadingAnchor(
  container: HTMLDivElement,
): ReadingAnchor | null {
  const containerTop = container.getBoundingClientRect().top;
  const safeTop = containerTop + READING_ANCHOR_SAFE_MARGIN_PX;
  const rows = container.querySelectorAll<HTMLElement>("[data-message-id]");
  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    if (rect.top >= safeTop) {
      const id = row.dataset.messageId;
      if (id)
        return {
          id,
          topOffset: rect.top - containerTop,
          scrollTop: container.scrollTop,
          row,
        };
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

/**
 * Layout-shift compensation for the reading anchor, computed scroll-invariantly.
 *
 * Given the anchor row's document position (`scrollTop + topOffset`) at baseline
 * and now, returns the absolute `scrollTop` the container should be written to
 * so the row stays visually fixed across a reflow above it — WITHOUT folding in
 * the user's own scroll motion since the baseline.
 *
 * The row's document position moves ONLY when content above it changes height:
 * a user scroll changes `scrollTop` and `topOffset` by equal-and-opposite
 * amounts and leaves the sum fixed. So `shift` (the reflow above the row) is the
 * change in document position, and the corrected target is `currentScrollTop +
 * shift`. When the user has purely scrolled (no reflow) the shift is 0 and the
 * target equals the current position — the correction ignores the wheel.
 *
 * Returns `null` when the shift is within `epsilonPx` (nothing to correct).
 */
export function computeAnchorCorrection(
  baseline: { topOffset: number; scrollTop: number },
  current: { topOffset: number; scrollTop: number },
  epsilonPx = 0.5,
): number | null {
  const shift =
    current.scrollTop +
    current.topOffset -
    (baseline.scrollTop + baseline.topOffset);
  if (Math.abs(shift) <= epsilonPx) return null;
  return current.scrollTop + shift;
}

/**
 * Net height change since the previous frame of the `.timeline-row-cv` rows in
 * the REALIZATION BAND above the anchor — rows whose document position is
 * between the top of the current viewport and the anchor's pre-reflow position.
 * Bounding to the band (not the whole above-anchor history) is load-bearing on
 * two counts:
 *
 *   - Correctness: only rows near the fold realize/reflow as the user scrolls
 *     up into them and thereby move the anchor *this frame*. Rows hundreds of px
 *     above scrolled past long ago; they quietly de-realize back toward their
 *     reserve as they leave the viewport, and summing that drift (which no walk
 *     re-synced) is exactly what pins `aboveShift` to a large bogus value.
 *   - Cost: the timeline is not DOM-virtualized — every message is a
 *     `.timeline-row-cv` — so an unbounded walk is O(channel) per realization
 *     frame. The band is viewport-sized, O(visible rows).
 *
 * Because the band is small and its rows are on-screen, this walk maintains the
 * height cache in place for band rows: a row's `last` is refreshed to its
 * current height every walk, so `height - last` is the single-frame reflow. A
 * band row's first sighting is seeded from its `contain-intrinsic-size` reserve
 * so a realization counts as its true `realized - reserve` delta (see
 * `reservedRowHeight`). Rows outside the band are neither read nor written.
 *
 * The iteration is bounded to the band, not just the sum: we start at the
 * anchor's own `.timeline-row-cv` and walk PRECEDING rows in document order via
 * a `TreeWalker`, stopping the moment a row falls below the band floor. Because
 * rows are laid out top-to-bottom in document order, everything before that
 * floor is older still, so the break is safe. This avoids the O(channel)
 * `querySelectorAll(".timeline-row-cv")` enumeration every frame — critical now
 * that the walk runs on every mid-history frame, not only realization frames.
 *
 * It is the SECOND, independent instrument the rAF writer cross-checks against
 * the anchor's net document-position shift (`computeAnchorCorrection`): the two
 * agree only when the net shift is genuinely an above-anchor reflow, not a
 * straddling-row miscount or scroll artifact.
 *
 * DEFERRED REFRESH: the cache write is staged, not applied inline — the walk
 * returns `{ aboveShift, commit }` and the caller applies `commit()` ONLY when
 * it keeps this frame (does not momentum-skip). This preserves "first observer
 * wins, second no-ops" while letting the momentum gate — which now needs
 * `aboveShift` to decide — skip a frame WITHOUT refreshing the cache, so the
 * next quiet frame still sees the pending reflow. Refreshing on a skip would
 * silently swallow it.
 */
export function sumAboveAnchorShift(
  container: HTMLElement,
  // The anchor row's own `.timeline-row-cv` wrapper — the walk's start node. We
  // step to its PRECEDING rows; the anchor itself is at the anchor position by
  // definition and never counts toward the above-anchor shift.
  anchorRow: HTMLElement,
  // The anchor's document position BEFORE this frame's reflow
  // (`baseline.scrollTop + baseline.topOffset`). A row moved the anchor iff it
  // sat above the anchor's *pre-realization* position; classifying by the
  // post-realization position miscounts a boundary row that realized up to
  // straddle the anchor (it didn't move the anchor, but ends up above it).
  anchorDocTop: number,
  // The current viewport's document top (`container.scrollTop`). The band's
  // lower bound is one row-reserve above it so a row straddling the top fold —
  // whose realization still shifts the anchor — is included.
  scrollTop: number,
  heights: WeakMap<Element, number>,
): { aboveShift: number; commit: () => void } {
  const wrapper = anchorRow.closest<HTMLElement>(".timeline-row-cv");
  if (!wrapper) return { aboveShift: 0, commit: () => {} };
  const containerTop = container.getBoundingClientRect().top;
  const bandTop = scrollTop - REFLOW_BAND_ABOVE_FOLD_PX;
  // Document-order walk over `.timeline-row-cv` rows, structure-agnostic: rows
  // are nested under day-group `<section>`s, so a plain sibling walk can't cross
  // group boundaries — `TreeWalker` does, and stays O(band).
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_ELEMENT, {
    acceptNode: (node) =>
      (node as HTMLElement).classList.contains("timeline-row-cv")
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_SKIP,
  });
  walker.currentNode = wrapper;
  let aboveShift = 0;
  // Staged cache writes, applied by `commit()` only when the caller keeps this
  // frame (does not momentum-skip). Includes first-sighting seeds so a skipped
  // frame does not seed either — the next quiet frame does the whole pass.
  const staged: Array<[Element, number]> = [];
  for (
    let row = walker.previousNode() as HTMLElement | null;
    row;
    row = walker.previousNode() as HTMLElement | null
  ) {
    const rect = row.getBoundingClientRect();
    // Row's document position = viewport-relative top + scrollTop, compared
    // against document-coord bounds so scroll between frames cancels.
    const rowDocTop = rect.top - containerTop + scrollTop;
    if (rowDocTop >= anchorDocTop) continue; // at/below anchor: doesn't move it.
    if (rowDocTop < bandTop) break; // older than the band; all prior are too.
    const height = rect.height;
    const last = heights.get(row);
    staged.push([row, height]);
    if (last === undefined) continue; // first sighting in band: seed, don't count.
    aboveShift += height - last;
  }
  const commit = () => {
    for (const [row, height] of staged) heights.set(row, height);
  };
  return { aboveShift, commit };
}

/**
 * Result of an attempted mid-history correction, surfaced for the E2E gate's
 * would-fire tripwires (Chromium rAF would-fire must be 0; WebKit RO fires must
 * be no-ops against the refreshed cache). `wouldFire` is true when both
 * instruments agreed on a real above-anchor reflow this call; `residual` is the
 * `|aboveShift|` the walk saw — the second observer of the same realization
 * sees this at ~0 because the first observer's walk already refreshed the cache.
 *
 * `signedShift` is `aboveShift` WITHOUT the abs — diagnostic-only, read by the
 * slow-scroll classifier. Its sign is the grow/shrink discriminator the escape
 * counter cannot recover from magnitude alone: `> 0` = content above grew, the
 * anchor was pushed down and the correction WRITE is the felt backward snap
 * (absorption's amortizable topology); `< 0` = content above shrank, the reflow
 * itself pulls the anchor up and renders the reversal BEFORE any write touches
 * it (structurally uncorrectable by us — only smaller per-frame realization
 * helps). `residual = |signedShift|` throws that sign away, so the classifier
 * reads `signedShift` directly. Not consumed in production.
 */
type MidHistoryCorrection = {
  wouldFire: boolean;
  residual: number;
  signedShift: number;
  // Diagnostic-only, rAF path only: the painted wheel motion the momentum gate
  // keyed on this frame (`aboveShift − ΔtopOffset`). Lets the classifier fixture
  // PROVE the scroll/reflow decomposition holds — a pure-scroll frame reads
  // large here, a pure-reflow (rendered-still) frame reads ~0. The RO path has
  // no momentum gate, so it omits this. Not consumed in production.
  renderedScroll?: number;
};

/**
 * The single mid-history correction, shared verbatim by BOTH the per-rAF
 * sampler and the ResizeObserver callback. Which one actually issues the write
 * on a given engine is NOT decided by an engine branch — it falls out of the
 * frame lifecycle (rAF → layout → RO → paint) plus the shared height cache:
 *
 *   - On Chromium the on-time RO delivers the realization pre-paint, so the RO
 *     call runs the walk first, corrects, and refreshes the band cache. The
 *     next rAF's walk then sees residual ≈ 0 and does nothing (wouldFire=false).
 *   - On WebKit the RO delivers one frame late, so the rAF call runs the walk
 *     first, corrects, refreshes; when the late RO finally fires it sees the
 *     refreshed cache → residual ≈ 0 → no-op.
 *
 * "First observer wins, second observer no-ops" is therefore implicit in the
 * cache, not coordinated by a flag. `sumAboveAnchorShift` reads the band and
 * STAGES its refresh, applied by `commit()` only past the momentum gate, so a
 * kept correction and its cache refresh are one indivisible pass — the second
 * observer cannot double-correct because the delta it would sum is already 0.
 *
 * `baseline` is the anchor's PRE-realization snapshot (the rAF frame-start read
 * of the reading row). We re-measure that same row NOW (post-layout) and diff.
 * All height reads are `getBoundingClientRect().height` — one clock, matching
 * the band walk — so no sub-pixel basis disagreement leaves a phantom residual.
 */
function applyMidHistoryCorrection(
  container: HTMLElement,
  baseline: ReadingAnchor,
  heights: WeakMap<Element, number>,
): MidHistoryCorrection {
  const currentScrollTop = container.scrollTop;
  const containerTop = container.getBoundingClientRect().top;
  const currentTopOffset =
    baseline.row.getBoundingClientRect().top - containerTop;
  const current = { topOffset: currentTopOffset, scrollTop: currentScrollTop };
  // The band walk computes `aboveShift` and STAGES the band-cache refresh; we
  // apply the refresh (`commit()`) only past the momentum gate, so a skip does
  // not swallow the pending reflow (see `sumAboveAnchorShift`).
  const { aboveShift, commit } = sumAboveAnchorShift(
    container,
    baseline.row,
    baseline.scrollTop + baseline.topOffset,
    currentScrollTop,
    heights,
  );
  const residual = Math.abs(aboveShift);
  // Momentum in flight: the two reads may not describe one coherent state, so
  // skip rather than fold the wheel into the correction. We gate on RENDERED
  // scroll, NOT the raw `scrollTop` delta: WebKit coalesces momentum into
  // `scrollTop` on its own async clock, so a frame that PAINTED still can read a
  // large raw delta and wrongly skip a genuine reflow (the survivor bin). The
  // rendered scroll is the anchor's painted move (`ΔtopOffset`) with the
  // reflow's own push removed — `renderedScroll = aboveShift − ΔtopOffset`,
  // both terms from the painted DOM — so a still frame reads ~0 and corrects.
  const renderedScroll = aboveShift - (currentTopOffset - baseline.topOffset);
  if (Math.abs(renderedScroll) > COMPENSATION_SCROLL_SKIP_PX) {
    return {
      wouldFire: false,
      residual,
      signedShift: aboveShift,
      renderedScroll,
    };
  }
  // Past the gate: keep this frame, so refresh the band cache now (zeroes the
  // second observer; first-observer-wins stays intact).
  commit();
  // Net document-position shift of the anchor since baseline (scroll-invariant),
  // and the gated correction target — same math, epsilon gate, as the unit-
  // tested `computeAnchorCorrection`.
  const observedShift =
    currentScrollTop +
    currentTopOffset -
    (baseline.scrollTop + baseline.topOffset);
  const target = computeAnchorCorrection(baseline, current);
  if (target === null)
    return {
      wouldFire: false,
      residual,
      signedShift: aboveShift,
      renderedScroll,
    };
  // Fire only when the two instruments agree — sufficiency cross-check that the
  // net shift is a real above-anchor reflow, not a straddler miscount.
  if (Math.abs(aboveShift - observedShift) > 0.5) {
    return {
      wouldFire: false,
      residual,
      signedShift: aboveShift,
      renderedScroll,
    };
  }
  // Synchronous setter (not `scrollTo`, which WebKit may defer past paint).
  container.scrollTop = target;
  return { wouldFire: true, residual, signedShift: aboveShift, renderedScroll };
}

/**
 * Build stamp for the E2E gate's stale-`dist` guard. `pnpm build` is
 * `tsc && vite build`; on a tsc failure it leaves the PRIOR `dist/` in place, so
 * a fixture can silently exercise a stale bundle and report a fabricated pass.
 * The fixture asserts this exact value is present on `window` after load, which
 * catches BOTH failure modes: "build failed, stale dist" (stamp absent, probe
 * never ran) and "build succeeded but I'm serving the previous experiment's
 * dist" (stamp present but not equal to the value the fixture expects). Bump
 * this string whenever the correction mechanism under test changes so a stale
 * bundle can never masquerade as the current experiment.
 */
const ANCHOR_BUILD_STAMP = "w4a-gate-1";

/**
 * Test-only tripwire hook. In production `window.__ANCHOR_PROBE__` is undefined
 * and this is a single truthiness check per correction attempt — no allocation,
 * no cost. The E2E gate installs the array and asserts the ratified invariants
 * from it: Chromium's on-time RO is the sole mid-history writer (`source==="ro"
 * && wouldFire` count > 0 AND `source==="raf" && wouldFire` count == 0); WebKit's
 * late RO no-ops against the rAF-refreshed cache (rAF fires AND every
 * `source==="ro" && wouldFire` entry has residual ≤ 0.5). One record per
 * attempt, both observers. On the first record we also stamp
 * `window.__ANCHOR_BUILD_STAMP__` so the fixture can prove it loaded THIS
 * build's bundle, not a stale one.
 */
function reportCorrection(
  source: "raf" | "ro",
  result: MidHistoryCorrection,
): void {
  const probe = (
    globalThis as unknown as {
      __ANCHOR_PROBE__?: Array<{
        source: "raf" | "ro";
        wouldFire: boolean;
        residual: number;
        signedShift: number;
        renderedScroll?: number;
      }>;
      __ANCHOR_BUILD_STAMP__?: string;
    }
  ).__ANCHOR_PROBE__;
  if (probe) {
    (
      globalThis as unknown as { __ANCHOR_BUILD_STAMP__?: string }
    ).__ANCHOR_BUILD_STAMP__ = ANCHOR_BUILD_STAMP;
    probe.push({
      source,
      wouldFire: result.wouldFire,
      residual: result.residual,
      signedShift: result.signedShift,
      renderedScroll: result.renderedScroll,
    });
  }
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
  // the viewport (top at/below the scroller top), its top offset, and the
  // scrollTop at capture. Holds the PREVIOUS frame's snapshot: the rAF sampler
  // reads it as the pre-realization baseline, then overwrites it with this
  // frame's snapshot (see the rAF sampler). Sampled every rAF by a running loop
  // while mid-history — NOT per-scroll-event, because scroll events dispatch
  // async off WebKit's scrolling thread and would hand a stale snapshot. rAF
  // callbacks run in the frame's rendering steps before layout on every engine,
  // and the sampler's synchronous read forces this frame's realization into
  // layout, so the pair (prev, this frame) spans the reflow.
  const readingAnchorRef = React.useRef<ReadingAnchor | null>(null);
  // Last-known laid-out height per observed `.timeline-row-cv` row, hoisted to
  // component scope so BOTH the ResizeObserver effect (which observes/seeds it)
  // and the per-rAF sampler (which owns the mid-history correction and reads it
  // to attribute per-row reflow) share one cache. The compensable delta of a
  // realization is `realized - reserve`, so each row is seeded at its
  // `contain-intrinsic-size` reserve (see `reservedRowHeight`); the rAF walk
  // then reads the realized height and diffs. If this cache lived in the RO
  // closure the rAF walk would have no baseline and would silently no-op.
  const rowHeightsRef = React.useRef<WeakMap<Element, number>>(new WeakMap());

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
  // Content resize while AT BOTTOM: a bottom-pinned in-viewport reflow (image
  // decode, embed expand, late font) or a row realizing grows `scrollHeight`
  // without a `messages` change, so the layout effect doesn't fire. The RO
  // callback runs in the rendering steps AFTER layout and BEFORE paint, which
  // makes a `scrollTo` here same-frame invisible — this is the correct trigger
  // for bottom-glue, not the async-dispatched `contentvisibilityautostatechange`
  // event (which may fire after the shifted frame has already painted).
  //
  // This effect owns ONLY bottom-glue and maintaining the shared row-height
  // cache. Mid-history realization correction moved to the per-rAF sampler
  // below: on WebKit the RO for a realization delivers one frame LATE (paint at
  // N, RO at N+1), so a correction issued here lands after the shifted frame
  // has painted — the visible row snap. The rAF sampler's synchronous
  // `getBoundingClientRect` forces the realization into its OWN frame's layout,
  // so it observes and corrects the shift same-frame, before paint. Keeping RO
  // as a second scroll writer would reintroduce the two-callback fight; RO
  // writes only the bottom floor.
  // ---------------------------------------------------------------------------
  // biome-ignore lint/correctness/useExhaustiveDependencies: `messages` is an intentional re-sync trigger — on each committed render we (re)observe any newly-mounted `.timeline-row-cv` rows so a row appended by a load-older page starts being watched. The callback reads only stable refs; `channelId` forces a full re-subscribe when the keyed scroll container remounts.
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    // Shared component-scope height cache (see `rowHeightsRef`). Seeded here at
    // observe time with each row's `contain-intrinsic-size` reserve so the rAF
    // walk reads a true `realized - reserve` delta on realization rather than
    // swallowing it as an unmeasurable first sighting.
    const lastHeights = rowHeightsRef.current;
    const observer = new ResizeObserver((entries) => {
      const container = scrollContainerRef.current;
      if (!container) return;
      // A programmatic bottom pin is still settling; `onScroll` owns the
      // floor-chase, so stay out of its way and don't double-write.
      if (settlingRef.current) return;
      // Only bottom-glue lives here. Mid-history is the rAF sampler's job.
      // Bottom vs mid-history is decided by SYNCHRONOUS geometry, not
      // `anchorRef.current.kind` (scroll-event-maintained → stale under WebKit
      // momentum). `isAtBottomNow` reads live scroll metrics, and it matches the
      // exact signal the rAF baseline sampler uses to decide whether a reading
      // anchor exists — so the branch here and the baseline can't disagree.
      if (isAtBottomNow(container)) {
        // Bottom-glue: a row realizing/reflowing while pinned grows the content,
        // so re-pin to the new floor to stay glued, and refresh the height cache
        // for the resized rows so the rAF walk doesn't later treat this
        // already-absorbed growth as a mid-history delta after the user scrolls
        // up. Partitioned from mid-history by `isAtBottomNow` — at-bottom and
        // mid-history are disjoint, so this scrollTo and the mid-history one
        // below can never both fire in one callback.
        for (const entry of entries) {
          lastHeights.set(
            entry.target,
            entry.target.getBoundingClientRect().height,
          );
        }
        container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
        return;
      }
      // Mid-history: the RO is the on-time observer on Chromium (delivers the
      // realization pre-paint), so it is the mid-history corrector THERE. On
      // WebKit the RO is late — by the time it fires the per-rAF sampler has
      // already corrected this realization and refreshed the height cache, so
      // the entry deltas below read ~0 (`changed` stays false) and this branch
      // no-ops. "First observer wins" falls out of the frame lifecycle + the
      // shared cache, with no engine branch.
      //
      // This corrector is UNGATED by the rAF path's `observedShift`/`aboveShift`
      // agreement cross-check, and deliberately so: that cross-check exists to
      // suppress the rAF band walk's straddler-miscount fabrication (see
      // `applyMidHistoryCorrection` and commit history), a risk the RO does not
      // have — the RO entries ARE ground truth for which rows resized. Applying
      // the cross-check here strangled the on-time Chromium observer (its box-
      // growth and the anchor's shove land one frame apart on a pipelined
      // engine, so the same-frame equality never held) and regressed Chromium
      // to 16 reversals. So: RO entries are the TRIGGER; correct by the anchor's
      // own measured drift; refresh the cache in the same callback.
      //
      // Baseline is the rAF's frame-start (pre-realization) snapshot of the
      // reading row, produced by `snapshotReadingAnchor` — the first FULLY
      // visible row held `READING_ANCHOR_SAFE_MARGIN_PX` below the fold. That
      // safe-margin anchor is the straddler guard on THIS path: because the
      // anchor sits a notch below the realization band, a row realizing across
      // the top fold is above the anchor and its delta is summed into the drift
      // correctly by the layout engine — it never corrupts the anchor's own
      // position. The agreement gate is the rAF band walk's straddler guard
      // (that walk sums per-row deltas and can fabricate); the RO path's guard
      // is the anchor margin, not the gate. Load-bearing invariant (Eva's
      // design-of-record (c), Quinn's merge-bar checklist #4): a refactor that
      // re-points this at a bare top-crossing anchor reintroduces the Shape-B
      // lurch on Chromium — the `changed` trigger does NOT cover straddlers, the
      // margin does. If no baseline yet (before the first rAF), skip.
      const baseline = readingAnchorRef.current;
      if (!baseline) return;
      // Trigger + refresh: did any observed row's laid-out height actually
      // change this batch? Refresh the cache to the realized height as we go
      // (same `getBoundingClientRect().height` basis as the rAF walk — one
      // clock) so a late WebKit RO for a realization the rAF already handled
      // sees a zero delta and this branch stays inert.
      let changed = false;
      for (const entry of entries) {
        const row = entry.target as HTMLElement;
        const height = row.getBoundingClientRect().height;
        const last = lastHeights.get(row);
        lastHeights.set(row, height);
        if (last === undefined) continue; // first sighting: seed, don't count.
        if (Math.abs(height - last) > 0.5) changed = true;
      }
      if (!changed) {
        reportCorrection("ro", {
          wouldFire: false,
          residual: 0,
          signedShift: 0,
        });
        return;
      }
      // Correct from the anchor's own measured drift. The layout engine already
      // summed every above-anchor height delta into the anchor row's top, and
      // rows resizing below the anchor don't move it — so the single measured
      // drift IS the net above-anchor shift, no per-row summation needed.
      const containerTop = container.getBoundingClientRect().top;
      const currentTopOffset =
        baseline.row.getBoundingClientRect().top - containerTop;
      const drift = currentTopOffset - baseline.topOffset;
      if (Math.abs(drift) <= 0.5) {
        reportCorrection("ro", {
          wouldFire: false,
          residual: Math.abs(drift),
          signedShift: drift,
        });
        return;
      }
      // Synchronous setter (not `scrollTo`, which WebKit may defer past paint).
      container.scrollTop = container.scrollTop + drift;
      // Re-baseline so a second RO batch this frame measures from where we
      // pinned, not the pre-correction position.
      readingAnchorRef.current = snapshotReadingAnchor(container);
      reportCorrection("ro", {
        wouldFire: true,
        residual: Math.abs(drift),
        signedShift: drift,
      });
    });
    // Observe every timeline row (not the content wrapper): a
    // `content-visibility: auto` row realizing to its true height is a resize
    // of THAT row's box but does not reliably fire a ResizeObserver on the
    // wrapper (Blink does not surface CV realization as an ancestor resize).
    // The RO callback runs after layout, before paint, so the compensating
    // scroll write is same-frame invisible.
    for (const row of content.querySelectorAll<HTMLElement>(
      ".timeline-row-cv",
    )) {
      lastHeights.set(row, reservedRowHeight(row));
      observer.observe(row);
    }
    return () => observer.disconnect();
  }, [channelId, contentRef, scrollContainerRef, messages]);

  // ---------------------------------------------------------------------------
  // Per-rAF reading-anchor sampler AND the sole mid-history scroll writer.
  //
  // rAF callbacks run in every engine's frame rendering steps BEFORE
  // style/layout, and the synchronous `getBoundingClientRect` in
  // `snapshotReadingAnchor` forces THIS frame's layout — including any
  // `content-visibility` realization — into the read. So the sampler observes a
  // realization same-frame, and a `scrollTo` issued here lands before the frame
  // paints. That is the whole cross-engine fix: on WebKit the ResizeObserver for
  // the same realization delivers one frame LATE (paint N, RO N+1), so an
  // RO-driven correction snaps visibly; correcting in the rAF that forced the
  // layout collapses N+1 → N. On Chromium the same rAF read sees the realization
  // same-frame too, so the single writer is correct on both engines.
  //
  // Two instruments, read from ONE forced-layout pass so they describe the SAME
  // realization (never a late-carried prior one):
  //   1. `observedShift` — the anchor row's net document-position delta since
  //      the previous frame's (pre-realization) snapshot. Cheap: two snapshots,
  //      no row walk. Scroll-invariant (`scrollTop + topOffset`): the user's own
  //      scroll moves both equal-and-opposite, so this is the reflow alone.
  //   2. `aboveShift` — the per-row-attributed sum of height deltas for rows
  //      laid out ABOVE the anchor (`sumAboveAnchorShift`), against the shared
  //      height cache. This is the SUFFICIENCY cross-check: `observedShift`
  //      alone moves for any reason (a straddling row miscount, an anchor-row
  //      top-edge resize), so we correct only when the two AGREE — that is the
  //      evidence the net shift is a genuine above-anchor reflow, not the
  //      "Shape B" straddler lurch. Losing this cross-check is exactly the W1
  //      sufficiency gap; it survives here because both reads are same-frame.
  //
  // Cost: the row walk runs ONLY when `|observedShift| > ε` — i.e. on
  // realization frames, the same frequency the RO fired at — so there is no
  // steady-state per-frame draw cost.
  //
  // Guards ported from the old RO path (both load-bearing):
  //   - Re-pick guard `prev.id === cur.id`: `snapshotReadingAnchor` re-selects
  //     the anchor by geometry every frame, so without this the shift would diff
  //     two DIFFERENT rows on a re-pick frame (constant during scroll).
  //   - Staleness skip: a large `scrollTop` delta since the previous frame means
  //     momentum is in flight and the two reads may not describe one coherent
  //     state — skip rather than fold the wheel into the pin.
  //
  // Mid-history is derived from SYNCHRONOUS geometry every frame
  // (`isAtBottomNow`), NOT from `anchorRef.current.kind` (scroll-event
  // maintained → stale under WebKit momentum). When at-bottom we clear the
  // anchor (bottom-glue in the RO effect owns that path); while a programmatic
  // bottom pin is settling we hold off — `onScroll` owns that window.
  //
  // No `channelId` dep: the loop reads `scrollContainerRef.current` fresh every
  // frame, so it re-binds to the new scroller on channel switch on its own.
  React.useEffect(() => {
    let rafId = requestAnimationFrame(function sample() {
      const container = scrollContainerRef.current;
      if (container && !settlingRef.current) {
        // `prev` is last frame's snapshot (pre-realization); take this frame's
        // snapshot into `cur`. The read forces layout, so `cur` reflects this
        // frame's realization — the pair (prev, cur) spans the reflow.
        const prev = readingAnchorRef.current;
        const cur = isAtBottomNow(container)
          ? null
          : snapshotReadingAnchor(container);
        readingAnchorRef.current = cur;

        // rAF is ONE of the two mid-history observers (see
        // `applyMidHistoryCorrection`). We attempt a correction every frame we
        // have a coherent prev→cur pair on the SAME anchor row (re-pick guard):
        // the band walk inside runs every frame to keep its cache single-frame
        // fresh, and issues the write only when both instruments agree. On
        // WebKit the rAF is the first observer (late RO) and this fires; on
        // Chromium the on-time RO already corrected + refreshed the cache last
        // step, so the walk here sees residual ≈ 0 and no-ops. `prev` is the
        // pre-realization baseline; the helper re-measures `prev.row` now.
        if (cur && prev && prev.id === cur.id) {
          const result = applyMidHistoryCorrection(
            container,
            prev,
            rowHeightsRef.current,
          );
          reportCorrection("raf", result);
        }
      }
      rafId = requestAnimationFrame(sample);
    });
    return () => cancelAnimationFrame(rafId);
  }, [scrollContainerRef]);

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
