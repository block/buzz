import * as React from "react";

const GESTURE_QUIET_MS = 180;
const UP_KEYS = new Set(["ArrowUp", "PageUp", "Home"]);

/** Scroll/layout callbacks cannot create reader intent. One gesture can consume
 * at most one history transaction, even when a fast page has already settled.
 */
export function useHistoryBoundaryIntent(
  hostRef: React.RefObject<HTMLDivElement | null>,
  onStartReached: (() => boolean) | undefined,
  armMomentum: (started: boolean) => void,
) {
  const stateRef = React.useRef({
    lastInput: -Infinity,
    consumed: false,
    eligible: false,
    pointerDown: false,
    touchDown: false,
  });
  const startRef = React.useRef(onStartReached);
  startRef.current = onStartReached;
  const tryStart = React.useCallback(() => {
    const state = stateRef.current;
    if (
      !state.eligible ||
      state.consumed ||
      (!state.pointerDown &&
        performance.now() - state.lastInput >= GESTURE_QUIET_MS)
    )
      return;
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement) || scroller.scrollTop > 200)
      return;
    if (startRef.current?.()) {
      state.consumed = true;
      armMomentum(true);
    }
  }, [armMomentum, hostRef]);
  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    let frame = 0;
    const input = (upward: boolean, fresh = false) => {
      const state = stateRef.current;
      const now = performance.now();
      if (
        fresh ||
        (!state.pointerDown &&
          !state.touchDown &&
          now - state.lastInput >= GESTURE_QUIET_MS)
      )
        state.consumed = false;
      state.lastInput = now;
      state.eligible = upward;
      cancelAnimationFrame(frame);
      // Let default scrolling happen first. This also covers upward input at
      // scrollTop=0, where no scroll event is emitted at all.
      frame = requestAnimationFrame(tryStart);
    };
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey) input(event.deltaY < 0);
    };
    const key = (event: KeyboardEvent) => {
      if (
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        !(event.target instanceof HTMLElement) ||
        event.target.closest("input,textarea,select,[contenteditable='true']")
      )
        return;
      if (UP_KEYS.has(event.key) || (event.key === " " && event.shiftKey))
        input(true, !event.repeat);
      else if (["ArrowDown", "PageDown", "End", " "].includes(event.key))
        input(false);
    };
    let previousOffset = 0;
    const pointer = (event: PointerEvent) => {
      if (event.target !== scroller || event.pointerType === "touch") return;
      stateRef.current.pointerDown = true;
      previousOffset = scroller.scrollTop;
      // A press in empty space is not upward intent. Only subsequent upward
      // scrollbar travel can consume this drag's transaction.
      input(false, true);
    };
    const pointerEnd = () => {
      stateRef.current.pointerDown = false;
      stateRef.current.eligible = false;
    };
    const scroll = () => {
      if (stateRef.current.pointerDown) {
        stateRef.current.eligible = scroller.scrollTop < previousOffset;
        previousOffset = scroller.scrollTop;
      }
      tryStart();
    };
    let touchY = 0;
    const touchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY ?? 0;
      stateRef.current.touchDown = true;
      stateRef.current.consumed = false;
      stateRef.current.eligible = false;
      stateRef.current.lastInput = performance.now();
    };
    const touchMove = (event: TouchEvent) => {
      const nextY = event.touches[0]?.clientY ?? touchY;
      input(nextY > touchY);
      touchY = nextY;
    };
    const touchEnd = () => {
      stateRef.current.touchDown = false;
    };
    const touchCancel = () => {
      touchEnd();
      stateRef.current.eligible = false;
    };
    scroller.addEventListener("wheel", wheel, { passive: true });
    scroller.addEventListener("keydown", key);
    scroller.addEventListener("pointerdown", pointer, { passive: true });
    window.addEventListener("pointerup", pointerEnd, { passive: true });
    window.addEventListener("pointercancel", pointerEnd, { passive: true });
    scroller.addEventListener("scroll", scroll, { passive: true });
    scroller.addEventListener("touchstart", touchStart, { passive: true });
    scroller.addEventListener("touchmove", touchMove, { passive: true });
    scroller.addEventListener("touchend", touchEnd, { passive: true });
    scroller.addEventListener("touchcancel", touchCancel, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      scroller.removeEventListener("wheel", wheel);
      scroller.removeEventListener("keydown", key);
      scroller.removeEventListener("pointerdown", pointer);
      window.removeEventListener("pointerup", pointerEnd);
      window.removeEventListener("pointercancel", pointerEnd);
      scroller.removeEventListener("scroll", scroll);
      scroller.removeEventListener("touchstart", touchStart);
      scroller.removeEventListener("touchmove", touchMove);
      scroller.removeEventListener("touchend", touchEnd);
      scroller.removeEventListener("touchcancel", touchCancel);
    };
  }, [hostRef, tryStart]);
  return tryStart;
}
