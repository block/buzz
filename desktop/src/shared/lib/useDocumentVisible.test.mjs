import assert from "node:assert/strict";
import { afterEach, describe, it, mock } from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import {
  isAppFocused,
  isDocumentVisible,
  subscribeAppFocus,
  subscribeDocumentVisibility,
  useFocusedRefetchInterval,
} from "./useDocumentVisible.ts";
import { useNow } from "./useNow.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;
const originalHTMLElement = globalThis.HTMLElement;
const originalActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT;

afterEach(() => {
  mock.timers.reset();
  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
  if (originalHTMLElement === undefined) delete globalThis.HTMLElement;
  else globalThis.HTMLElement = originalHTMLElement;
  if (originalActEnvironment === undefined)
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  else globalThis.IS_REACT_ACT_ENVIRONMENT = originalActEnvironment;
});

function installVisibilityStub() {
  let visibilityState = "visible";
  let focused = true;
  const documentListeners = new Map();
  const windowListeners = new Map();
  globalThis.document = {
    get visibilityState() {
      return visibilityState;
    },
    hasFocus: () => focused,
    addEventListener(type, listener) {
      const listeners = documentListeners.get(type) ?? new Set();
      listeners.add(listener);
      documentListeners.set(type, listeners);
    },
    removeEventListener(type, listener) {
      const listeners = documentListeners.get(type);
      listeners?.delete(listener);
      if (listeners?.size === 0) documentListeners.delete(type);
    },
  };
  globalThis.window = {
    addEventListener(type, listener) {
      const listeners = windowListeners.get(type) ?? new Set();
      listeners.add(listener);
      windowListeners.set(type, listeners);
    },
    removeEventListener(type, listener) {
      const listeners = windowListeners.get(type);
      listeners?.delete(listener);
      if (listeners?.size === 0) windowListeners.delete(type);
    },
  };
  return {
    documentListeners,
    windowListeners,
    setFocused(value) {
      focused = value;
    },
    setVisibility(value) {
      visibilityState = value;
    },
  };
}

describe("document visibility", () => {
  it("defaults to visible and focused when document is unavailable", () => {
    delete globalThis.document;
    assert.equal(isDocumentVisible(), true);
    assert.equal(isAppFocused(), true);
  });

  it("keeps true visibility separate from app focus", () => {
    const stub = installVisibilityStub();
    const visibleStates = [];
    const focusedStates = [];
    const unsubscribeVisibility = subscribeDocumentVisibility((visible) => {
      visibleStates.push(visible);
    });
    const unsubscribeFocus = subscribeAppFocus((focused) => {
      focusedStates.push(focused);
    });

    stub.setFocused(false);
    for (const listener of stub.windowListeners.get("blur")) listener();
    assert.equal(isDocumentVisible(), true);
    assert.equal(isAppFocused(), false);
    assert.deepEqual(visibleStates, []);
    assert.deepEqual(focusedStates, [false]);

    stub.setFocused(true);
    for (const listener of stub.windowListeners.get("focus")) listener();
    stub.setVisibility("hidden");
    for (const listener of stub.documentListeners.get("visibilitychange"))
      listener();
    assert.deepEqual(visibleStates, [false]);
    assert.deepEqual(focusedStates, [false, true, false]);

    unsubscribeVisibility();
    unsubscribeFocus();
    assert.equal(stub.documentListeners.size, 0);
    assert.equal(stub.windowListeners.size, 0);
  });
});

describe("visibility-gated hooks", () => {
  it("useNow pauses while hidden and snaps fresh on return", async () => {
    mock.timers.enable({ apis: ["Date", "setInterval"], now: 1_000 });
    const dom = new JSDOM(
      "<!doctype html><html><body><div id='root'></div></body></html>",
    );
    let visibilityState = "visible";
    Object.defineProperty(dom.window.document, "visibilityState", {
      configurable: true,
      get: () => visibilityState,
    });
    Object.assign(globalThis, {
      document: dom.window.document,
      HTMLElement: dom.window.HTMLElement,
      IS_REACT_ACT_ENVIRONMENT: true,
      window: dom.window,
    });
    const observed = [];
    function Harness() {
      const now = useNow(1_000);
      React.useEffect(() => {
        observed.push(now);
      }, [now]);
      return null;
    }
    const root = createRoot(document.getElementById("root"));

    await act(async () => root.render(React.createElement(Harness)));
    await act(async () => mock.timers.tick(1_000));
    visibilityState = "hidden";
    await act(async () =>
      document.dispatchEvent(new window.Event("visibilitychange")),
    );
    await act(async () => mock.timers.tick(5_000));
    assert.equal(observed.at(-1), 2_000);

    visibilityState = "visible";
    await act(async () =>
      document.dispatchEvent(new window.Event("visibilitychange")),
    );
    assert.equal(observed.at(-1), 7_000);

    await act(async () => root.unmount());
    dom.window.close();
  });

  it("focused polling pauses on blur and refreshes immediately on focus", async () => {
    const dom = new JSDOM(
      "<!doctype html><html><body><div id='root'></div></body></html>",
    );
    let focused = true;
    Object.defineProperty(dom.window.document, "hasFocus", {
      configurable: true,
      value: () => focused,
    });
    Object.defineProperty(dom.window.document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
    Object.assign(globalThis, {
      document: dom.window.document,
      HTMLElement: dom.window.HTMLElement,
      IS_REACT_ACT_ENVIRONMENT: true,
      window: dom.window,
    });
    const observed = [];
    function Harness() {
      const refetchInterval = useFocusedRefetchInterval(1_000);
      React.useEffect(() => {
        observed.push(refetchInterval);
      }, [refetchInterval]);
      return null;
    }
    const root = createRoot(document.getElementById("root"));

    await act(async () => root.render(React.createElement(Harness)));
    focused = false;
    await act(async () => window.dispatchEvent(new window.Event("blur")));
    assert.deepEqual(observed, [1_000, false]);
    focused = true;
    await act(async () => window.dispatchEvent(new window.Event("focus")));
    assert.deepEqual(observed, [1_000, false, 1_000]);

    await act(async () => root.unmount());
    dom.window.close();
  });
});
