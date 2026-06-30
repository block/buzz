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

function canScrollY(element: HTMLElement, deltaY: number) {
  if (deltaY < 0) {
    return element.scrollTop > BOUNDARY_EPSILON_PX;
  }

  const maxScrollTop = element.scrollHeight - element.clientHeight;
  return element.scrollTop < maxScrollTop - BOUNDARY_EPSILON_PX;
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
 * bands the entire app and reveals a blank strip above/below the UI. CSS
 * `overscroll-behavior` is not enough for all of the empty/header/footer hit
 * targets in the webview, so this capture listener consumes only gestures that
 * otherwise have nowhere app-local to scroll.
 *
 * Real scrolling is left alone: if any scroll container under the pointer can
 * move in the wheel direction, the browser handles it normally. At boundaries,
 * only containers marked with `data-buzz-conversation-scroll` are allowed to
 * receive the gesture so their own local elastic affordance can remain; every
 * other boundary is locked and cannot chain to the viewport.
 */
export function useWebviewScrollBoundaryLock() {
  React.useEffect(() => {
    function handleWheel(event: WheelEvent) {
      if (event.defaultPrevented || event.deltaY === 0 || event.ctrlKey) {
        return;
      }

      const path = event.composedPath();
      let firstScrollable: HTMLElement | null = null;

      for (const target of path) {
        if (!isHTMLElement(target) || !isScrollableY(target)) {
          continue;
        }

        firstScrollable ??= target;
        if (canScrollY(target, event.deltaY)) {
          return;
        }
      }

      if (firstScrollable && isConversationScroller(firstScrollable)) {
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
