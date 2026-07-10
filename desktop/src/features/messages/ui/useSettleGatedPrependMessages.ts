import * as React from "react";

/**
 * Holds an older-history prepend out of the rendered timeline until the
 * scroller is genuinely at rest, then admits it atomically.
 *
 * Why: every prepend-compensation mechanism in this design — Virtua's shift
 * correction, the pre-paint scrollBy, the semantic-anchor watcher — is a
 * scrollTop write. On macOS WKWebView those writes can be dropped or
 * overridden while trackpad momentum owns the committed offset, so a page
 * commit that lands mid-fling displaces the viewport by the full prepended
 * height with no reliable way to correct it. Committing only at rest keeps
 * all three writers operating in the regime where they are exact.
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
 * Upper bound on how long a fetched page may be withheld. Trackpad momentum
 * decays in well under a second; a reader actively driving the scroller for
 * this long has moved on, and admitting under continuous REAL input is safe —
 * the dropped-write hazard is specific to the inertial momentum phase, which
 * cannot outlive this deadline.
 */
export const SETTLE_HOLD_DEADLINE_MS = 4_000;

export type SettleGateDecision<T> =
  | { kind: "pass" }
  | { kind: "hold"; held: T[] };

/**
 * Pure admission rule. Holds only a PURE history prepend: the next snapshot
 * must be the admitted rows (same ids, possibly refreshed objects — edits and
 * reactions keep rendering) with one or more new rows in front. Anything else
 * — appends, deletions, authoritative replacements, channel resets — passes
 * through immediately so the gate can never pin a stale dataset.
 */
export function selectSettleGatedMessages<T extends { id: string }>({
  admitted,
  next,
}: {
  admitted: readonly T[];
  next: T[];
}): SettleGateDecision<T> {
  if (admitted.length === 0) return { kind: "pass" };
  const prependCount = next.findIndex(
    (message) => message.id === admitted[0].id,
  );
  if (prependCount <= 0) return { kind: "pass" };
  if (next.length - prependCount !== admitted.length) return { kind: "pass" };
  for (let index = 0; index < admitted.length; index += 1) {
    if (next[prependCount + index].id !== admitted[index].id) {
      return { kind: "pass" };
    }
  }
  return { kind: "hold", held: next.slice(prependCount) };
}

export function useSettleGatedPrependMessages<T extends { id: string }>({
  channelId,
  messages,
  scrollElementRef,
}: {
  channelId?: string | null;
  messages: T[];
  scrollElementRef: { readonly current: HTMLElement | null };
}): { messages: T[]; isHoldingPrepend: boolean } {
  const admittedRef = React.useRef<T[]>(messages);
  const previousChannelIdRef = React.useRef(channelId);
  const [, admit] = React.useReducer((epoch: number) => epoch + 1, 0);

  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    admittedRef.current = messages;
  }

  const decision = selectSettleGatedMessages({
    admitted: admittedRef.current,
    next: messages,
  });
  const isHoldingPrepend = decision.kind === "hold";

  let output: T[];
  if (decision.kind === "hold") {
    // Same ids as the admitted set; keep array identity stable unless a row
    // object actually changed so Virtua's data model is not rebuilt per render.
    const previous = admittedRef.current;
    output =
      previous.length === decision.held.length &&
      previous.every((message, index) => message === decision.held[index])
        ? previous
        : decision.held;
  } else {
    output = messages;
  }
  admittedRef.current = output;

  const latestMessagesRef = React.useRef(messages);
  latestMessagesRef.current = messages;

  React.useEffect(() => {
    if (!isHoldingPrepend) return;
    const scroller = scrollElementRef.current;
    if (!scroller) {
      // Nothing to observe — never strand the fetched page.
      admittedRef.current = latestMessagesRef.current;
      admit();
      return;
    }
    let frame: number | null = null;
    const deadline = performance.now() + SETTLE_HOLD_DEADLINE_MS;
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
    const watch = () => {
      const scrollTop = scroller.scrollTop;
      settledFrames =
        Math.abs(scrollTop - previousScrollTop) < 0.5 ? settledFrames + 1 : 0;
      previousScrollTop = scrollTop;
      const quiet = performance.now() - lastMotionTs >= SETTLE_MOTION_WINDOW_MS;
      if (
        (quiet && settledFrames >= SETTLE_FRAME_COUNT) ||
        performance.now() >= deadline
      ) {
        frame = null;
        admittedRef.current = latestMessagesRef.current;
        admit();
        return;
      }
      frame = requestAnimationFrame(watch);
    };
    frame = requestAnimationFrame(watch);
    return () => {
      scroller.removeEventListener("scroll", markMotion);
      scroller.removeEventListener("wheel", markMotion);
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [isHoldingPrepend, scrollElementRef]);

  return { messages: output, isHoldingPrepend };
}
