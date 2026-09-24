/**
 * Shared JSDOM setup for the canvas history tests that mount
 * CanvasHistoryPanel, whose Restore action opens a Radix AlertDialog. Radix's
 * focus/dismiss machinery reaches for DOM-level globals without a `window.`
 * prefix (getComputedStyle, NodeFilter, the HTML/SVG constructors, ...) and
 * its layer coordination dispatches plain objects through dispatchEvent, which
 * JSDOM's strict Event validation rejects. This installs both so the dialog's
 * effects settle under bare `node:test` + jsdom.
 *
 * Call once from a test's `before()` after constructing the JSDOM instance and
 * before importing React. Additive to the basic globals (document, window,
 * HTMLElement, navigator, matchMedia, localStorage) each test already sets.
 */
export function installRadixDialogGlobals(dom) {
  globalThis.self = dom.window;
  globalThis.MutationObserver = dom.window.MutationObserver;
  if (!globalThis.ResizeObserver) {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  }
  dom.window.ResizeObserver = globalThis.ResizeObserver;
  dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;

  // Bulk-copy the DOM-level globals Radix references without a `window.`
  // prefix. Bulk copy avoids per-internal whack-a-mole.
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (
      !(key in globalThis) &&
      (key.startsWith("HTML") ||
        key.startsWith("SVG") ||
        [
          "Node",
          "NodeFilter",
          "NodeList",
          "Event",
          "CustomEvent",
          "MouseEvent",
          "KeyboardEvent",
          "FocusEvent",
          "PointerEvent",
          "EventTarget",
          "DocumentFragment",
          "getComputedStyle",
        ].includes(key))
    ) {
      const val = dom.window[key];
      if (val !== undefined) globalThis[key] = val;
    }
  }
  // getComputedStyle must stay bound to dom.window or it throws "Illegal
  // invocation" when Radix calls it.
  globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
  // Radix DismissableLayer/FocusScope dispatch plain objects through
  // dispatchEvent for layer coordination; JSDOM's strict Event validation
  // throws on those. Drop non-Event objects so the dialog's effects settle
  // without affecting real Event delivery.
  const origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
  dom.window.EventTarget.prototype.dispatchEvent = function (event) {
    if (!(event instanceof dom.window.Event)) return false;
    return origDispatch.call(this, event);
  };
  globalThis.EventTarget = dom.window.EventTarget;
}
