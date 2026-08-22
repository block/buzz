import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

// `pretendToBeVisual` is what gives jsdom `requestAnimationFrame`. The drawer
// defers its focus restore to one, so without it the restore silently never
// runs and every assertion here would pass for the wrong reason.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  pretendToBeVisual: true,
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    Node: dom.window.Node,
    // The drawer calls the bare global, not `window.requestAnimationFrame`.
    cancelAnimationFrame: dom.window.cancelAnimationFrame,
    document: dom.window.document,
    requestAnimationFrame: dom.window.requestAnimationFrame,
    window: dom.window,
  });
  // `navigator` is getter-only on Node, so it needs defineProperty rather than
  // assignment; motion/react reads it during render.
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  // `useReducedMotion` subscribes to a media query jsdom does not implement.
  dom.window.matchMedia ??= () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

/**
 * The drawer defers its focus restore to a `requestAnimationFrame`, so every
 * assertion here has to run after that frame has actually fired. Two hops:
 * React commits the unmount, then the rAF callback runs.
 */
async function flushDeferredFocusRestore(act) {
  for (let hop = 0; hop < 2; hop += 1) {
    await act(async () => {
      await new Promise((resolve) =>
        dom.window.requestAnimationFrame(() => resolve()),
      );
    });
  }
}

async function loadHarness() {
  const React = await import("react");
  const { act, render } = await import("@testing-library/react");
  const { CoverDrawer } = await import("./CoverDrawer.tsx");
  const { releaseCoverDrawerFocus } = await import(
    "@/features/channels/lib/coverDrawerFocusSlot"
  );

  const opener = dom.window.document.createElement("button");
  opener.setAttribute("data-testid", "opener");
  dom.window.document.body.append(opener);
  opener.focus();
  assert.equal(dom.window.document.activeElement, opener);

  const drawer = (testId) =>
    React.createElement(
      CoverDrawer,
      { ariaLabel: testId, onClose: () => {}, scrimLabel: testId, testId },
      React.createElement("div", null, testId),
    );

  return { act, drawer, opener, releaseCoverDrawerFocus, render };
}

function activeTestId() {
  return (
    dom.window.document.activeElement?.getAttribute("data-testid") ??
    dom.window.document.activeElement?.tagName ??
    "none"
  );
}

test("a drawer restores focus to whatever it covered when it simply closes", async () => {
  const { act, drawer, opener, render } = await loadHarness();
  const view = render(drawer("first-drawer"));
  assert.equal(activeTestId(), "first-drawer");

  await act(async () => {
    view.unmount();
  });
  await flushDeferredFocusRestore(act);

  assert.equal(dom.window.document.activeElement, opener);
});

test("a replaced drawer leaves focus with its successor, not the covered content", async () => {
  // The bug this guards: restoration is deferred a frame, and in that frame the
  // successor has already mounted and taken focus. An unconditional restore
  // yanks focus back out of the new drawer and into content that is now inert —
  // keyboard-dead, with no visible focus ring anywhere on screen.
  const { act, drawer, render } = await loadHarness();
  const view = render(drawer("outgoing-drawer"));

  // Replacement, with no fully-closed intermediate state: the successor mounts
  // and claims focus while the outgoing drawer is still animating out.
  const successor = render(drawer("incoming-drawer"));
  assert.equal(activeTestId(), "incoming-drawer");
  await act(async () => {
    view.unmount();
  });
  await flushDeferredFocusRestore(act);

  assert.equal(activeTestId(), "incoming-drawer");
  successor.unmount();
});

test("only the last-opened drawer keeps focus across a chain of replacements", async () => {
  const { act, drawer, render } = await loadHarness();
  const first = render(drawer("first-drawer"));
  const second = render(drawer("second-drawer"));
  const third = render(drawer("third-drawer"));

  await act(async () => {
    first.unmount();
    second.unmount();
  });
  await flushDeferredFocusRestore(act);

  assert.equal(activeTestId(), "third-drawer");
  third.unmount();
});

test("a released slot leaves focus where the caller put it", async () => {
  // The thread view-mode switch: nothing replaces the drawer, but the switch has
  // already decided where focus belongs, so the drawer's own restore must not
  // fire and drag focus back to whatever opened the thread.
  const { act, drawer, releaseCoverDrawerFocus, render } = await loadHarness();
  const view = render(drawer("thread-drawer"));

  const splitPane = dom.window.document.createElement("button");
  splitPane.setAttribute("data-testid", "split-pane");
  dom.window.document.body.append(splitPane);

  await act(async () => {
    releaseCoverDrawerFocus();
    splitPane.focus();
    view.unmount();
  });
  await flushDeferredFocusRestore(act);

  assert.equal(activeTestId(), "split-pane");
});
