import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    MouseEvent: dom.window.MouseEvent,
    MutationObserver: dom.window.MutationObserver,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  // jsdom doesn't implement these — BrowserSurface's bounds/visibility
  // effect schedules its own recompute via rAF.
  const raf = (callback) => setTimeout(() => callback(Date.now()), 0);
  const caf = (id) => clearTimeout(id);
  globalThis.requestAnimationFrame = raf;
  globalThis.cancelAnimationFrame = caf;
  dom.window.requestAnimationFrame = raf;
  dom.window.cancelAnimationFrame = caf;
});

after(() => dom.window.close());

test("an old route's pending command rejection never affects the new route after a key-based remount", async () => {
  const { act, cleanup, render, waitFor } = await import(
    "@testing-library/react"
  );
  const React = await import("react");
  const { clearMocks, mockIPC } = await import("@tauri-apps/api/mocks");
  const { emit } = await import("@tauri-apps/api/event");
  const { closeActiveBrowserSession } = await import(
    "@/features/plugins/useBrowserSession.ts"
  );
  const { PluginBrowserRoute } = await import(
    "./plugins.$pluginId.$contributionId.tsx"
  );

  let rejectStaleBack;
  const staleBackInvoke = new Promise((_resolve, reject) => {
    rejectStaleBack = reject;
  });

  const sessions = {
    "p1:c1": {
      sessionId: "session-1",
      generation: 1,
      homeUrl: "https://example.com/home1",
    },
    "p2:c2": {
      sessionId: "session-2",
      generation: 1,
      homeUrl: "https://example.com/home2",
    },
  };

  mockIPC(
    (command, args) => {
      switch (command) {
        case "plugin_browser_open":
          return sessions[`${args.pluginId}:${args.contributionId}`];
        case "plugin_browser_close":
        case "plugin_browser_set_bounds":
        case "plugin_browser_set_visible":
          return null;
        case "plugin_browser_back":
          // Only ever dispatched against the old (p1/c1) instance in this
          // test — its rejection is delivered deliberately late, after the
          // remount below.
          return staleBackInvoke;
        default:
          throw new Error(`unexpected command: ${command}`);
      }
    },
    { shouldMockEvents: true },
  );

  try {
    const { container, rerender } = render(
      React.createElement(PluginBrowserRoute, {
        pluginId: "p1",
        contributionId: "c1",
      }),
    );

    await waitFor(() => {
      const address = container.querySelector("[aria-label='Address']");
      assert.equal(address?.value, sessions["p1:c1"].homeUrl);
    });

    // Make Back clickable on the old (p1/c1) instance.
    await act(async () => {
      await emit("plugin-browser-location", {
        sessionId: sessions["p1:c1"].sessionId,
        generation: sessions["p1:c1"].generation,
        url: sessions["p1:c1"].homeUrl,
        canGoBack: true,
        canGoForward: false,
      });
    });

    const backButton = container.querySelector("[aria-label='Back']");
    assert.ok(backButton && !backButton.disabled);
    await act(async () => {
      backButton.dispatchEvent(
        new dom.window.MouseEvent("click", { bubbles: true }),
      );
    });

    // Navigate away: a key-based remount, exactly what the real route does
    // when pluginId/contributionId change.
    await act(async () => {
      rerender(
        React.createElement(PluginBrowserRoute, {
          pluginId: "p2",
          contributionId: "c2",
        }),
      );
    });

    await waitFor(() => {
      const address = container.querySelector("[aria-label='Address']");
      assert.equal(address?.value, sessions["p2:c2"].homeUrl);
    });

    // The stale Back command from the OLD (p1/c1) instance settles only now,
    // well after the new (p2/c2) instance has taken over — it must not
    // touch the new instance's chrome state.
    await act(async () => {
      rejectStaleBack(new Error("stale session closed"));
      await staleBackInvoke.catch(() => {});
    });

    assert.equal(
      container.querySelector("[role='alert']"),
      null,
      "the new route must show no error from the old route's stale rejection",
    );
    const address = container.querySelector("[aria-label='Address']");
    assert.equal(address.value, sessions["p2:c2"].homeUrl);
  } finally {
    cleanup();
    try {
      // `cleanup()`'s own fire-and-forget closeActiveBrowserSession() on
      // unmount (see useBrowserSession.ts) is a floating promise queued on
      // the module-level serialized lifecycle — await the same exported
      // queue directly so it's actually drained before clearMocks() removes
      // the mock it needs, rather than guessing at a timer delay. Bounded
      // so a stuck queue fails this test instead of hanging the whole file.
      let clearRaceTimeout = () => {};
      await Promise.race([
        closeActiveBrowserSession(),
        new Promise((_resolve, reject) => {
          const timer = setTimeout(() => {
            reject(new Error("closeActiveBrowserSession teardown timed out"));
          }, 2_000);
          clearRaceTimeout = () => clearTimeout(timer);
        }),
      ]);
      clearRaceTimeout();
    } finally {
      // Must run even if the teardown await above rejects/times out, or a
      // stuck close leaks the mock into every test file that runs after
      // this one.
      clearMocks();
    }
  }
});
