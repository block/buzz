import * as React from "react";

const MIN_THUMB_HEIGHT = 24;
const IDLE_FADE_DELAY_MS = 700;

type UserSelectStyle = {
  userSelect: string;
};

let bodySelectionLockCount = 0;
let bodySelectionLockStyle: UserSelectStyle | null = null;
let bodySelectionPreviousValue = "";

/**
 * Disables body text selection until every overlapping scrollbar drag releases
 * its lock. The returned release function is safe to call more than once.
 */
export function acquireBodySelectionLock(
  style: UserSelectStyle = document.body.style,
): () => void {
  if (bodySelectionLockCount === 0) {
    bodySelectionLockStyle = style;
    bodySelectionPreviousValue = style.userSelect;
    style.userSelect = "none";
  }
  bodySelectionLockCount += 1;

  let isReleased = false;
  return () => {
    if (isReleased) return;
    isReleased = true;
    bodySelectionLockCount -= 1;

    if (bodySelectionLockCount === 0) {
      if (bodySelectionLockStyle) {
        bodySelectionLockStyle.userSelect = bodySelectionPreviousValue;
      }
      bodySelectionLockStyle = null;
      bodySelectionPreviousValue = "";
    }
  };
}

export type OverlayScrollbarGeometry = {
  maxThumbOffset: number;
  scrollRange: number;
  thumbHeight: number;
  thumbOffset: number;
  trackHeight: number;
};

type OverlayScrollbarGeometryInput = {
  bottomInset: number;
  clientHeight: number;
  scrollHeight: number;
  scrollTop: number;
};

type OverlayScrollbarDragInput = {
  deltaY: number;
  dragStartScrollTop: number;
  maxThumbOffset: number;
  scrollRange: number;
};

export function calculateOverlayScrollbarGeometry({
  bottomInset,
  clientHeight,
  scrollHeight,
  scrollTop,
}: OverlayScrollbarGeometryInput): OverlayScrollbarGeometry | null {
  const scrollRange = scrollHeight - clientHeight;
  const trackHeight = clientHeight - Math.max(0, bottomInset);
  if (scrollRange <= 0 || trackHeight <= MIN_THUMB_HEIGHT) {
    return null;
  }

  const thumbHeight = Math.min(
    trackHeight,
    Math.max(MIN_THUMB_HEIGHT, (clientHeight / scrollHeight) * trackHeight),
  );
  const maxThumbOffset = trackHeight - thumbHeight;
  const clampedScrollTop = Math.min(Math.max(scrollTop, 0), scrollRange);
  const thumbOffset = (clampedScrollTop / scrollRange) * maxThumbOffset;

  return {
    maxThumbOffset,
    scrollRange,
    thumbHeight,
    thumbOffset,
    trackHeight,
  };
}

export function calculateScrollTopFromThumbDrag({
  deltaY,
  dragStartScrollTop,
  maxThumbOffset,
  scrollRange,
}: OverlayScrollbarDragInput): number {
  if (maxThumbOffset <= 0 || scrollRange <= 0) {
    return Math.min(Math.max(dragStartScrollTop, 0), Math.max(scrollRange, 0));
  }

  const scrollDelta = (deltaY / maxThumbOffset) * scrollRange;
  return Math.min(Math.max(dragStartScrollTop + scrollDelta, 0), scrollRange);
}

type UseOverlayScrollbarOptions = {
  composerRef: React.RefObject<HTMLElement | null>;
  resetKey?: unknown;
  scrollRef: React.RefObject<HTMLElement | null>;
  thumbRef: React.RefObject<HTMLDivElement | null>;
};

export function useOverlayScrollbar({
  composerRef,
  resetKey,
  scrollRef,
  thumbRef,
}: UseOverlayScrollbarOptions) {
  React.useEffect(() => {
    void resetKey;
    const scrollElement = scrollRef.current;
    const thumbElement = thumbRef.current;
    if (!scrollElement || !thumbElement) return;

    let fadeTimer: ReturnType<typeof setTimeout> | null = null;
    let isDragging = false;
    let isHovering = false;
    let dragPointerId: number | null = null;
    let dragStartY = 0;
    let dragStartScrollTop = 0;
    let releaseBodySelectionLock: (() => void) | null = null;

    const clearFadeTimer = () => {
      if (fadeTimer !== null) {
        globalThis.clearTimeout(fadeTimer);
        fadeTimer = null;
      }
    };

    const hideThumb = () => {
      clearFadeTimer();
      thumbElement.style.opacity = "0";
      thumbElement.style.pointerEvents = "none";
    };

    const updateGeometry = (): OverlayScrollbarGeometry | null => {
      const geometry = calculateOverlayScrollbarGeometry({
        bottomInset: composerRef.current?.getBoundingClientRect().height ?? 0,
        clientHeight: scrollElement.clientHeight,
        scrollHeight: scrollElement.scrollHeight,
        scrollTop: scrollElement.scrollTop,
      });
      if (!geometry) {
        hideThumb();
        return null;
      }

      thumbElement.style.height = `${geometry.thumbHeight}px`;
      thumbElement.style.transform = `translateY(${geometry.thumbOffset}px)`;
      thumbElement.style.pointerEvents = "auto";
      return geometry;
    };

    const scheduleFade = () => {
      clearFadeTimer();
      if (isDragging || isHovering) return;
      fadeTimer = globalThis.setTimeout(() => {
        thumbElement.style.opacity = "0";
        fadeTimer = null;
      }, IDLE_FADE_DELAY_MS);
    };

    const showThumb = () => {
      if (!updateGeometry()) return;
      thumbElement.style.opacity = "1";
      scheduleFade();
    };

    const restoreSelection = () => {
      releaseBodySelectionLock?.();
      releaseBodySelectionLock = null;
    };

    const finishDrag = (pointerId: number) => {
      if (!isDragging || pointerId !== dragPointerId) return;
      isDragging = false;
      dragPointerId = null;
      if (thumbElement.hasPointerCapture(pointerId)) {
        thumbElement.releasePointerCapture(pointerId);
      }
      restoreSelection();
      scheduleFade();
    };

    const handleScroll = () => showThumb();
    const handlePointerEnter = () => {
      isHovering = true;
      clearFadeTimer();
      showThumb();
    };
    const handlePointerLeave = () => {
      isHovering = false;
      scheduleFade();
    };
    const handlePointerDown = (event: PointerEvent) => {
      if (isDragging || event.button !== 0 || !updateGeometry()) return;
      event.preventDefault();
      event.stopPropagation();
      isDragging = true;
      dragPointerId = event.pointerId;
      dragStartY = event.clientY;
      dragStartScrollTop = scrollElement.scrollTop;
      releaseBodySelectionLock = acquireBodySelectionLock();
      thumbElement.setPointerCapture(event.pointerId);
      clearFadeTimer();
      thumbElement.style.opacity = "1";
    };
    const handlePointerMove = (event: PointerEvent) => {
      if (!isDragging || event.pointerId !== dragPointerId) return;
      const geometry = updateGeometry();
      if (!geometry || geometry.maxThumbOffset <= 0) return;
      scrollElement.scrollTop = calculateScrollTopFromThumbDrag({
        deltaY: event.clientY - dragStartY,
        dragStartScrollTop,
        maxThumbOffset: geometry.maxThumbOffset,
        scrollRange: geometry.scrollRange,
      });
      thumbElement.style.opacity = "1";
    };
    const handlePointerUp = (event: PointerEvent) =>
      finishDrag(event.pointerId);
    const handleLostPointerCapture = (event: PointerEvent) =>
      finishDrag(event.pointerId);

    scrollElement.addEventListener("scroll", handleScroll, { passive: true });
    // Match the sidebar scrollbar: entering anywhere in the scrollable pane
    // reveals the thumb, so a short thumb does not need to be found while it
    // is transparent.
    scrollElement.addEventListener("pointerenter", handlePointerEnter);
    scrollElement.addEventListener("pointerleave", handlePointerLeave);
    thumbElement.addEventListener("pointerenter", handlePointerEnter);
    thumbElement.addEventListener("pointerleave", handlePointerLeave);
    thumbElement.addEventListener("pointerdown", handlePointerDown);
    thumbElement.addEventListener("pointermove", handlePointerMove);
    thumbElement.addEventListener("pointerup", handlePointerUp);
    thumbElement.addEventListener("pointercancel", handlePointerUp);
    thumbElement.addEventListener(
      "lostpointercapture",
      handleLostPointerCapture,
    );

    const resizeObserver = new ResizeObserver(showThumb);
    resizeObserver.observe(scrollElement);
    // Virtua owns measurement of its resizing inner content. Observing that
    // same node here disrupts its prepend-anchor delivery order.
    if (composerRef.current) {
      resizeObserver.observe(composerRef.current);
    }

    showThumb();

    return () => {
      clearFadeTimer();
      resizeObserver.disconnect();
      scrollElement.removeEventListener("scroll", handleScroll);
      scrollElement.removeEventListener("pointerenter", handlePointerEnter);
      scrollElement.removeEventListener("pointerleave", handlePointerLeave);
      thumbElement.removeEventListener("pointerenter", handlePointerEnter);
      thumbElement.removeEventListener("pointerleave", handlePointerLeave);
      thumbElement.removeEventListener("pointerdown", handlePointerDown);
      thumbElement.removeEventListener("pointermove", handlePointerMove);
      thumbElement.removeEventListener("pointerup", handlePointerUp);
      thumbElement.removeEventListener("pointercancel", handlePointerUp);
      thumbElement.removeEventListener(
        "lostpointercapture",
        handleLostPointerCapture,
      );
      restoreSelection();
    };
  }, [composerRef, resetKey, scrollRef, thumbRef]);
}
