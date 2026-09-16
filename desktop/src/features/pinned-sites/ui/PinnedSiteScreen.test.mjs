import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

import { PinnedSiteLoadError } from "./PinnedSiteScreen.tsx";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  dom.window.matchMedia = () => ({
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

test("PinnedSiteLoadError shows the failure and retries start_url", async () => {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  let retried = 0;
  render(
    createElement(PinnedSiteLoadError, {
      message: "The page did not load.",
      onRetry: () => {
        retried += 1;
      },
    }),
  );
  assert.ok(screen.getByTestId("pinned-site-load-error"));
  assert.match(
    screen.getByTestId("pinned-site-load-error").textContent ?? "",
    /The page did not load/,
  );
  screen.getByTestId("pinned-site-load-error-retry").click();
  assert.equal(retried, 1);
});

test("unmount hide and late show re-hide the pin webview", async () => {
  const invokes = [];
  let resolveShow;
  const showGate = new Promise((resolve) => {
    resolveShow = resolve;
  });
  const internals = {
    invoke(cmd, args) {
      invokes.push({ cmd, args });
      if (cmd === "pin_webview_show") {
        return showGate.then(() => ({
          canGoBack: false,
          canGoForward: false,
          currentUrl: args.startUrl,
        }));
      }
      return Promise.resolve(undefined);
    },
  };
  globalThis.isTauri = true;
  window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  window.__TAURI_INTERNALS__ = internals;

  const React = await import("react");
  const { render, cleanup } = await import("@testing-library/react");
  const {
    hidePinWebview,
    pinWebviewBoundsAreUsable,
    showPinWebview,
  } = await import("../lib/pinWebview.ts");

  function Harness() {
    const hostRef = React.useRef(null);
    React.useEffect(() => {
      let cancelled = false;
      let opened = false;
      const open = () => {
        if (cancelled) return;
        const bounds = { x: 10, y: 40, width: 640, height: 480 };
        if (!opened) {
          if (!pinWebviewBoundsAreUsable(bounds)) return;
          opened = true;
          const id = "pin-late";
          void showPinWebview({
            pinId: id,
            startUrl: "https://late.test",
            bounds,
          }).then(() => {
            if (cancelled) void hidePinWebview(id);
          });
        }
      };
      open();
      return () => {
        cancelled = true;
        void hidePinWebview("pin-late");
      };
    }, []);
    return React.createElement("div", { ref: hostRef });
  }

  render(React.createElement(Harness));
  // Allow the effect to schedule show before unmounting.
  await Promise.resolve();
  assert.ok(
    invokes.some((row) => row.cmd === "pin_webview_show"),
    "expected show to start before unmount",
  );
  cleanup();
  assert.ok(invokes.some((row) => row.cmd === "pin_webview_hide"));
  const hidesBefore = invokes.filter((row) => row.cmd === "pin_webview_hide").length;
  resolveShow();
  // Flush the deferred show → epoch/cancelled re-hide microtasks.
  await new Promise((resolve) => setTimeout(resolve, 0));
  const hidesAfter = invokes.filter((row) => row.cmd === "pin_webview_hide").length;
  assert.ok(
    hidesAfter > hidesBefore,
    `expected late re-hide, before=${hidesBefore} after=${hidesAfter} calls=${JSON.stringify(invokes)}`,
  );
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete window.isTauri;
  delete window.__TAURI_INTERNALS__;
});
