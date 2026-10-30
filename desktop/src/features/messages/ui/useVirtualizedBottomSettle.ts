import * as React from "react";
import type { VListHandle } from "virtua";

const SCROLL_INTENT_KEYS = new Set([
  "ArrowDown",
  "ArrowUp",
  "End",
  "Home",
  "PageDown",
  "PageUp",
  " ",
]);

function isEditableKeyboardTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return (
    target.closest("input, textarea, select, [contenteditable='true']") !== null
  );
}

/**
 * Calls `onReaderScroll` when the reader takes scroll ownership of
 * `scroller`: a wheel, a touch drag, a pointerdown on the scroller itself
 * (scrollbar or background), or a scroll key aimed at it. Returns the
 * unsubscribe function.
 */
export function listenForReaderScroll(
  scroller: HTMLElement,
  onReaderScroll: () => void,
): () => void {
  const onTouchMove = () => onReaderScroll();
  const onPointerDown = (event: PointerEvent) => {
    // A descendant pointerdown is ordinary row interaction (link, reaction,
    // thread action), not evidence that the reader took scroll ownership.
    // Direct scroller hits cover scrollbar/background drag initiation.
    if (event.target === scroller) onReaderScroll();
  };
  const onWheel = (event: WheelEvent) => {
    // Ctrl+wheel is browser zoom, not reader navigation. The resulting
    // viewport/content reflow must still settle at the new physical floor.
    if (!event.ctrlKey) onReaderScroll();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      event.target instanceof Node &&
      scroller.contains(event.target) &&
      !isEditableKeyboardTarget(event.target) &&
      SCROLL_INTENT_KEYS.has(event.key)
    ) {
      onReaderScroll();
    }
  };
  scroller.addEventListener("pointerdown", onPointerDown, { passive: true });
  scroller.addEventListener("touchmove", onTouchMove, { passive: true });
  scroller.addEventListener("wheel", onWheel, { passive: true });
  window.addEventListener("keydown", onKeyDown, true);
  return () => {
    scroller.removeEventListener("pointerdown", onPointerDown);
    scroller.removeEventListener("touchmove", onTouchMove);
    scroller.removeEventListener("wheel", onWheel);
    window.removeEventListener("keydown", onKeyDown, true);
  };
}

export function useVirtualizedBottomSettle(
  hostRef: React.RefObject<HTMLDivElement | null>,
  listRef: React.RefObject<VListHandle | null>,
  itemsLengthRef: React.RefObject<number>,
) {
  const bottomIntentRef = React.useRef(false);
  const frameRef = React.useRef<number | null>(null);

  const cancelFrame = React.useCallback(() => {
    if (frameRef.current !== null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
  }, []);

  const cancel = React.useCallback(() => {
    bottomIntentRef.current = false;
    cancelFrame();
  }, [cancelFrame]);

  const pinToBottom = React.useCallback(() => {
    if (!bottomIntentRef.current) return;
    const scroller = hostRef.current?.firstElementChild;
    const lastIndex = itemsLengthRef.current - 1;
    if (!(scroller instanceof HTMLDivElement) || lastIndex < 0) return;
    listRef.current?.scrollToIndex(lastIndex, { align: "end" });
  }, [hostRef, itemsLengthRef, listRef]);

  const schedulePinToBottom = React.useCallback(() => {
    if (!bottomIntentRef.current || frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      pinToBottom();
    });
  }, [pinToBottom]);

  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    return listenForReaderScroll(scroller, cancel);
  }, [cancel, hostRef]);

  React.useLayoutEffect(() => {
    const scroller = hostRef.current?.firstElementChild;
    if (!(scroller instanceof HTMLDivElement)) return;
    const content = scroller.firstElementChild;
    if (
      !(content instanceof HTMLElement) ||
      typeof ResizeObserver === "undefined"
    ) {
      return;
    }
    // Virtua updates this inner element's extent whenever measured row geometry
    // changes. Bottom intent therefore follows the physical floor for as long
    // as it remains active, rather than guessing that layout is done after an
    // arbitrary timeout. Reader input and explicit target/prepend navigation
    // retire the intent through `cancel`.
    const observer = new ResizeObserver(schedulePinToBottom);
    observer.observe(content);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [hostRef, schedulePinToBottom]);

  const settle = React.useCallback(() => {
    bottomIntentRef.current = true;
    cancelFrame();
    pinToBottom();
  }, [cancelFrame, pinToBottom]);

  React.useEffect(() => cancel, [cancel]);
  return { cancel, settle };
}
