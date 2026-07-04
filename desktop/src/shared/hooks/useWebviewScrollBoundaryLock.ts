import * as React from "react";

const BOUNDARY_EPSILON_PX = 1;
const CONVERSATION_SCROLL_SELECTOR = "[data-buzz-conversation-scroll]";
const SCROLLABLE_OVERFLOW_VALUES = new Set(["auto", "scroll", "overlay"]);

function isHTMLElement(value: EventTarget | null): value is HTMLElement {
  return value instanceof HTMLElement;
}

function isDocumentElement(element: HTMLElement) {
  return element === document.body || element === document.documentElement;
}

function isScrollableY(element: HTMLElement) {
  if (isDocumentElement(element)) {
    return false;
  }

  const style = window.getComputedStyle(element);
  if (!SCROLLABLE_OVERFLOW_VALUES.has(style.overflowY)) {
    return false;
  }

  return element.scrollHeight > element.clientHeight + BOUNDARY_EPSILON_PX;
}

function isScrollableX(element: HTMLElement) {
  if (isDocumentElement(element)) {
    return false;
  }

  const style = window.getComputedStyle(element);
  if (!SCROLLABLE_OVERFLOW_VALUES.has(style.overflowX)) {
    return false;
  }

  return element.scrollWidth > element.clientWidth + BOUNDARY_EPSILON_PX;
}

function canScrollY(element: HTMLElement, deltaY: number) {
  if (deltaY < 0) {
    return element.scrollTop > BOUNDARY_EPSILON_PX;
  }

  const maxScrollTop = element.scrollHeight - element.clientHeight;
  return element.scrollTop < maxScrollTop - BOUNDARY_EPSILON_PX;
}

function canScrollX(element: HTMLElement, deltaX: number) {
  if (deltaX < 0) {
    return element.scrollLeft > BOUNDARY_EPSILON_PX;
  }

  const maxScrollLeft = element.scrollWidth - element.clientWidth;
  return element.scrollLeft < maxScrollLeft - BOUNDARY_EPSILON_PX;
}

function isConversationScroller(element: HTMLElement) {
  return Boolean(element.closest(CONVERSATION_SCROLL_SELECTOR));
}

/**
 * Stops macOS/WKWebView rubber-band gestures from escaping into the viewport.
 *
 * Buzz is laid out as fixed-height nested panes. On macOS, a wheel/trackpad
 * gesture that starts over a non-scrollable pane (or over a scrollable pane at
 * its boundary) can still be handed to the WKWebView viewport, which rubber-
 * bands the entire app and reveals a blank strip beside the UI. CSS
 * `overscroll-behavior` is not enough for all of the empty/header/footer hit
 * targets in the webview, so this capture listener consumes only gestures that
 * otherwise have nowhere app-local to scroll. Both axes are locked: vertical
 * and horizontal pans are each checked against containers that can actually
 * move in that direction.
 *
 * Real scrolling is left alone: if any scroll container under the pointer can
 * move in the wheel direction, the browser handles it normally. At vertical
 * boundaries, only containers marked with `data-buzz-conversation-scroll` are
 * allowed to receive the gesture so their own local elastic affordance can
 * remain; every other boundary — including all horizontal ones — is locked and
 * cannot chain to the viewport.
 */
export function useWebviewScrollBoundaryLock() {
  React.useEffect(() => {
    function handleWheel(event: WheelEvent) {
      if (event.defaultPrevented || event.ctrlKey) {
        return;
      }

      const { deltaX, deltaY } = event;
      if (deltaX === 0 && deltaY === 0) {
        return;
      }

      const path = event.composedPath();
      let firstScrollable: HTMLElement | null = null;

      for (const target of path) {
        if (!isHTMLElement(target)) {
          continue;
        }

        const scrollableY = deltaY !== 0 && isScrollableY(target);
        const scrollableX = deltaX !== 0 && isScrollableX(target);
        if (!scrollableY && !scrollableX) {
          continue;
        }

        firstScrollable ??= target;
        if (
          (scrollableY && canScrollY(target, deltaY)) ||
          (scrollableX && canScrollX(target, deltaX))
        ) {
          return;
        }
      }

      // Only the vertical elastic affordance of conversation scrollers is
      // preserved; a predominantly horizontal gesture must never pan the
      // webview, even over a conversation pane.
      if (
        firstScrollable &&
        isConversationScroller(firstScrollable) &&
        Math.abs(deltaY) >= Math.abs(deltaX)
      ) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
    }

    window.addEventListener("wheel", handleWheel, {
      capture: true,
      passive: false,
    });
    return () => {
      window.removeEventListener("wheel", handleWheel, { capture: true });
    };
  }, []);
}
