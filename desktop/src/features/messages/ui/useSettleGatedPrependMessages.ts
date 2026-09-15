import * as React from "react";

/**
 * Holds an older-history prepend out of the rendered timeline until the
 * scroller is genuinely at rest, then admits it atomically.
 *
 * Why: Virtua reconciles an active prepend by correcting scrollTop as the
 * inserted rows are measured. On macOS WKWebView those corrections can be
 * dropped or overridden while trackpad momentum owns the committed offset, so
 * a page admitted mid-fling can displace the viewport by the full prepended
 * height. Admitting only at rest gives Virtua a stable geometry window in
 * which to complete that reconciliation exactly; subsequent reader wheel input
 * then retires it.
 *
 * The fetched store stays authoritative and fetches still start immediately;
 * this hook only delays when the fetched page joins the rendered snapshot.
 */

/**
 * WebKit can freeze the JS-readable scrollTop for ~2 frames during live
 * trackpad momentum, so counting zero-delta frames alone misreports "settled"
 * mid-fling. Settle therefore requires BOTH a quiet window since the last
 * scroll/wheel event AND stable frame-over-frame offsets — the same
 * two-signal settle shape ratified for the timeline settle gate elsewhere in
 * this codebase.
 */
export const SETTLE_MOTION_WINDOW_MS = 100;
export const SETTLE_FRAME_COUNT = 3;
/**
 * There is deliberately no unconditional admission deadline. A fresh gesture
 * can start at any age of the request; elapsed fetch time is not proof that
 * WebKit has relinquished momentum. The held page remains available and is
 * admitted as soon as real input and geometry settle.
 */

export type SettleGateDecision<T> =
  | { kind: "pass" }
  | { kind: "hold"; held: T[] };

/**
 * Hold a history prefix even when deletion, live output or row regrouping
 * accompanies it. Survivors may refresh while held, but structural metadata
 * stays at the admitted publication until motion stops. A disjoint replacement
 * is not history pagination and must not pin a stale channel.
 */
export function selectSettleGatedMessages<T extends { id: string }>({
  admitted,
  next,
}: {
  admitted: readonly T[];
  next: T[];
}): SettleGateDecision<T> {
  if (admitted.length === 0) return { kind: "pass" };
  const admittedIds = new Set(admitted.map((message) => message.id));
  const firstSurvivor = next.findIndex((message) =>
    admittedIds.has(message.id),
  );
  if (firstSurvivor <= 0) return { kind: "pass" };
  return {
    kind: "hold",
    held: next.filter((message) => admittedIds.has(message.id)),
  };
}

export function useSettleGatedPrependMessages<T extends { id: string }, M>({
  channelId,
  messages,
  meta,
  scrollElementRef,
  bypass = false,
}: {
  channelId?: string | null;
  messages: T[];
  /**
   * Snapshot metadata that must stay paired with the rows it was projected
   * from (e.g. the history-exhaustion proof that decides whether the oldest
   * day divider may exist). While a prepend is held, the output keeps the
   * ADMITTED metadata; rows and metadata only ever advance together, so no
   * render can pair new proof with withheld rows.
   */
  meta: M;
  scrollElementRef: { readonly current: HTMLElement | null };
  /** Explicit latest/send/navigation intent may release held history. */
  bypass?: boolean;
}): { messages: T[]; meta: M; isHoldingPrepend: boolean } {
  const admittedRef = React.useRef<T[]>(messages);
  const admittedMetaRef = React.useRef<M>(meta);
  const previousChannelIdRef = React.useRef(channelId);
  const [, admit] = React.useReducer((epoch: number) => epoch + 1, 0);

  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    admittedRef.current = messages;
    admittedMetaRef.current = meta;
  }

  const decision: SettleGateDecision<T> = bypass
    ? { kind: "pass" }
    : selectSettleGatedMessages({
        admitted: admittedRef.current,
        next: messages,
      });
  const isHoldingPrepend = decision.kind === "hold";

  let output: T[];
  let outputMeta: M;
  if (decision.kind === "hold") {
    // Same ids as the admitted set; keep array identity stable unless a row
    // object actually changed so Virtua's data model is not rebuilt per render.
    const previous = admittedRef.current;
    output =
      previous.length === decision.held.length &&
      previous.every((message, index) => message === decision.held[index])
        ? previous
        : decision.held;
    outputMeta = admittedMetaRef.current;
  } else {
    output = messages;
    outputMeta = meta;
  }
  admittedRef.current = output;
  admittedMetaRef.current = outputMeta;

  const latestMessagesRef = React.useRef(messages);
  latestMessagesRef.current = messages;
  const latestMetaRef = React.useRef(meta);
  latestMetaRef.current = meta;

  React.useEffect(() => {
    if (!isHoldingPrepend) return;
    const scroller = scrollElementRef.current;
    if (!scroller) {
      // Nothing to observe — never strand the fetched page.
      admittedRef.current = latestMessagesRef.current;
      admittedMetaRef.current = latestMetaRef.current;
      admit();
      return;
    }
    let frame: number | null = null;
    // Assume motion at hold start: worst case this costs one quiet window
    // (~100ms) behind the fetching-older spinner when the reader was already
    // at rest; the alternative admits mid-fling if WebKit starves the first
    // scroll events.
    let lastMotionTs = performance.now();
    let previousScrollTop = scroller.scrollTop;
    let settledFrames = 0;
    const markMotion = () => {
      lastMotionTs = performance.now();
    };
    scroller.addEventListener("scroll", markMotion, { passive: true });
    scroller.addEventListener("wheel", markMotion, { passive: true });
    scroller.addEventListener("touchmove", markMotion, { passive: true });
    scroller.addEventListener("keydown", markMotion);
    const watch = () => {
      const scrollTop = scroller.scrollTop;
      settledFrames =
        Math.abs(scrollTop - previousScrollTop) < 0.5 ? settledFrames + 1 : 0;
      previousScrollTop = scrollTop;
      const quiet = performance.now() - lastMotionTs >= SETTLE_MOTION_WINDOW_MS;
      if (quiet && settledFrames >= SETTLE_FRAME_COUNT) {
        frame = null;
        admittedRef.current = latestMessagesRef.current;
        admittedMetaRef.current = latestMetaRef.current;
        admit();
        return;
      }
      frame = requestAnimationFrame(watch);
    };
    frame = requestAnimationFrame(watch);
    return () => {
      scroller.removeEventListener("scroll", markMotion);
      scroller.removeEventListener("wheel", markMotion);
      scroller.removeEventListener("touchmove", markMotion);
      scroller.removeEventListener("keydown", markMotion);
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [isHoldingPrepend, scrollElementRef]);

  return { messages: output, meta: outputMeta, isHoldingPrepend };
}
